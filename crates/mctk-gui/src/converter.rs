//! The Converter page: INPUT -> OUTPUT -> OPTIONS -> CONVERT -> SAVE.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};
use mctk_convert::{ConvertOptions, Progress};
use mctk_registry::Registry;
use mctk_replace::{Choice, Plan, ReplacementMap};

use crate::editor::edition_chooser;
use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::settings::Settings;
use crate::theme;
use crate::widgets;
use crate::workspace::copy_dir;
use crate::world_picker::UiEdition;

/// A card of `size` centerd in the page, never starting above it. The card is as tall as its
/// content turned out to be last time (`ConverterPage::ui` measures it), if that is more than
/// `size`, so a card taller than expected is still centerd and not pushed off the bottom.
fn centered(ui: &Ui, size: Vec2) -> Rect {
    let area = ui.max_rect();
    let real: f32 = ui.ctx().data(|d| d.get_temp(egui::Id::new("converter-real-height"))).unwrap_or(0.0);
    let mut r = Rect::from_center_size(area.center(), vec2(size.x, size.y.max(real)));
    if r.top() < area.top() {
        r = r.translate(vec2(0.0, area.top() - r.top()));
    }
    // Where the card starts, so its own height is measured (not the room above it as well,
    // which made every card count as tall as the page, and drawn at its top).
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("converter-card-top"), r.top()));
    r
}

const STEPS: [&str; 5] = ["INPUT", "OUTPUT", "OPTIONS", "CONVERT", "SAVE"];
/// Java to Java adds a step where the user reviews what the target version cannot hold.
const STEPS_JAVA: [&str; 6] = ["INPUT", "OUTPUT", "OPTIONS", "REPLACE", "CONVERT", "SAVE"];

/// A dimension's chunks, for the conversion's picture: being read, read, or not readable.
enum Footprint {
    Reading(Job<Vec<(i32, i32)>>),
    Ready(Arc<Vec<(i32, i32)>>),
    Failed,
}

/// What the REPLACE step scanned: the plan and the registry its block search runs on.
struct Scanned {
    plan: Plan,
    registry: Registry,
}

/// Which Bedrock version the version list has chosen (an index into the built-in versions, newest first).
static BEDROCK_INDEX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub struct ConverterPage {
    step: usize,
    pub input: Option<(PathBuf, UiEdition)>,
    output: Option<UiEdition>,
    /// Index into the built-in Java versions chosen as the output.
    java_version: usize,
    /// The scan of the input world against the chosen Java version, and its background job.
    scanned: Option<Scanned>,
    scan_job: Option<Job<Scanned>>,
    scan_error: Option<String>,
    /// What the scan was made for, so it is redone when either changes.
    scan_for: Option<(PathBuf, usize, u8)>,
    /// Search text of the open replacement picker.
    search: String,
    dims: [bool; 3],
    skip_partial: bool,
    tiles: bool,
    mobs: bool,
    ticks: bool,
    older_chunks: bool,
    depth: usize,
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
    /// Room each step needed last time beyond its nominal card (see `ui`), so a step whose
    /// content grew (a long list of notes) is scaled to fit.
    real_height: [f32; 6],
    /// How wide the step cards are (wider in a wide window, so their text can be bigger without
    /// making them taller), and the height a step has before it scrolls.
    card_w: f32,
    room_h: f32,
    /// The height the REPLACE step's list was given last time (the rest of its card is what the
    /// card measured beyond it).
    replace_list_h: Option<f32>,
    replace_card_h: f32,
    /// The same for the Convert step's list of what was left out.
    notes_h: Option<f32>,
    convert_card_h: f32,
    /// The world's chunks by dimension, for the conversion's picture (see ootprint).
    footprints: std::collections::HashMap<String, Footprint>,
    /// For development: start the conversion as soon as the Convert step shows.
    dev_start: bool,
    /// The conversion's picture, as a texture (see chunk_preview::image).
    preview_tex: Option<egui::TextureHandle>,
    /// What the finished conversion was made from and for (see `choice_key`).
    converted_for: Option<String>,
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
            java_version: 0,
            scanned: None,
            scan_job: None,
            scan_error: None,
            scan_for: None,
            search: String::new(),
            dims: [c[0] != 0, c[1] != 0, c[2] != 0],
            skip_partial: c[3] != 0,
            tiles: true,
            mobs: true,
            ticks: true,
            older_chunks: true,
            depth: 0,
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
            real_height: [0.0; 6],
            card_w: 680.0,
            room_h: 470.0,
            replace_list_h: None,
            replace_card_h: 0.0,
            notes_h: None,
            convert_card_h: 0.0,
            footprints: Default::default(),
            dev_start: false,
            preview_tex: None,
            converted_for: None,
            status: String::new(),
        }
    }

    /// The world, edition and version a conversion is made from and for.
    fn choice_key(&self) -> String {
        format!(
            "{:?}|{:?}|{}|{}",
            self.input,
            self.output,
            self.java_version,
            BEDROCK_INDEX.load(std::sync::atomic::Ordering::Relaxed)
        )
    }

    /// For development: a Java world to convert to an older Java version, at its REPLACE step.
    pub fn dev_replace(&mut self, path: PathBuf, version: &str, step: usize) {
        self.input = Some((path, UiEdition::Java));
        self.output = Some(UiEdition::Java);
        self.java_version = Self::java_versions().iter().position(|v| *v == version).unwrap_or(0);
        self.step = step;
        // 6: the Convert step, started at once (a real conversion, to watch its picture).
        if step == 6 {
            self.step = 4;
            self.dev_start = true;
            return;
        }
        // The Save step: a finished conversion with a list of what was left out (a sample).
        if step >= 5 {
            let mut text = "4830".to_string();
            for (n, what) in [(3613, "partially generated chunks"), (73, "entities: minecraft:item"), (66, "entities: minecraft:falling_block"), (49, "block entities: minecraft:potent_sulfur"), (6, "block entities: minecraft:creaking_heart"), (4, "entities: minecraft:marker"), (2, "block entities: minecraft:test_block")] {
                text.push_str(&format!("\n{n}\u{d7} left out: {what}"));
            }
            self.result = Some(Ok(text));
            self.converted_for = Some(self.choice_key());
        }
    }

    /// Lets go of the world: back to choosing one. Not while it is being converted.
    pub fn reset(&mut self) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.step = 0;
        self.input = None;
        self.output = None;
        self.result = None;
        self.progress = None;
        self.scanned = None;
        self.scan_for = None;
        self.scan_error = None;
        self.converted_for = None;
        self.real_height = [0.0; 6];
        true
    }

    pub fn set_input(&mut self, path: PathBuf, edition: UiEdition) {
        self.input = Some((path, edition));
        self.output = None;
        self.step = 1;
    }

    /// Java to Java: the one route with a REPLACE step and a choice of output version.
    #[cfg(test)]
    fn java_to_java(&self) -> bool {
        self.input.as_ref().map(|i| i.1) == Some(UiEdition::Java) && self.output == Some(UiEdition::Java)
    }

    /// Java worlds converted to Java or Bedrock, and Bedrock worlds converted to Java, get a
    /// REPLACE step for what the target cannot hold.
    fn has_replace_step(&self) -> bool {
        match (self.input.as_ref().map(|i| i.1), self.output) {
            (Some(UiEdition::Java), Some(UiEdition::Java | UiEdition::Bedrock | UiEdition::Xbox360 | UiEdition::Ps3 | UiEdition::WiiU)) => true,
            // Bedrock worlds go through a Java world to a Bedrock version or a console game, and
            // what that one cannot hold is chosen against the Java world.
            (Some(UiEdition::Bedrock), Some(_)) => true,
            _ => false,
        }
    }

    /// The console game version the writer produces saves for.
    const CONSOLE_VERSION: &'static str = "TU73 (Latest)";

    /// The Bedrock version chosen in the version list (0 is the newest).
    fn bedrock_version() -> &'static str {
        let v = mctk_registry::builtin::bedrock_versions();
        v[BEDROCK_INDEX.load(std::sync::atomic::Ordering::Relaxed).min(v.len() - 1)]
    }

    /// Index of the CONVERT step; SAVE follows it.
    fn convert_idx(&self) -> usize {
        if self.has_replace_step() { 4 } else { 3 }
    }

    fn java_versions() -> Vec<&'static str> {
        mctk_registry::builtin::java_versions()
    }

    fn java_version_name(&self) -> &'static str {
        let v = Self::java_versions();
        v[self.java_version.min(v.len() - 1)]
    }

    /// Label under the output tile: the chosen Java version, or the newest Bedrock one.
    fn version_label(&self, edition: UiEdition) -> String {
        match edition {
            UiEdition::Java if self.java_version == 0 => format!("{} (Latest)", self.java_version_name()),
            UiEdition::Java => self.java_version_name().to_string(),
            UiEdition::Bedrock if BEDROCK_INDEX.load(std::sync::atomic::Ordering::Relaxed) == 0 => format!("{} (Latest)", Self::bedrock_version()),
            UiEdition::Bedrock => Self::bedrock_version().to_string(),
            _ => Self::CONSOLE_VERSION.to_string(),
        }
    }

    /// Output editions this build can write. Any input reaches Java or Bedrock: a route with no
    /// direct converter goes through a temporary Java world on the way (see `start`). Console
    /// saves cannot be written yet.
    fn supported(&self, _out: UiEdition) -> bool {
        self.input.is_some()
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
            tile_entities: self.tiles,
            entities: self.mobs,
            ticks: self.ticks,
            older_chunks: self.older_chunks,
            depth: match self.depth {
                1 => mctk_convert::depth::DepthMode::Clip,
                2 => mctk_convert::depth::DepthMode::Raise,
                _ => mctk_convert::depth::DepthMode::ClipBedrock,
            },
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
        let name = world_name(&src);
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
        let java_version = self.java_version_name();
        let replacements =self.scanned.as_ref().map(|s| s.plan.to_map()).unwrap_or_default();
        self.job = Some(Job::spawn_notifying("Converting", ctx, move || {
            route_job(ed, out_ed, &src, &target, java_version, &replacements, &options, &p)
        }));
        self.converted_for = Some(self.choice_key());
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
                self.step = self.convert_idx() + 1;
            }
            self.result = Some(r);
        }
        if let Some(r) = self.scan_job.as_ref().and_then(Job::take) {
            self.scan_job = None;
            match r {
                Ok(s) => self.scanned = Some(s),
                Err(e) => self.scan_error = Some(e),
            }
        }
        // The REPLACE step needs a scan of this world against this version.
        if self.has_replace_step() && self.step == 3 {
            self.ensure_scan(ui.ctx());
        }
        // A finished conversion stays while the user looks back at the steps. It is forgotten when
        // they choose another world to convert, or another edition or version to convert to.
        if self.job.is_none() && self.result.is_some() && self.converted_for.as_deref() != Some(self.choice_key().as_str()) {
            self.result = None;
            self.progress = None;
            self.elapsed = 0.0;
            self.converted_for = None;
            self.real_height = [0.0; 6];
        }
        let full = ui.max_rect();
        // A short window scales the stepper and the banner down.
        let k = (full.height() / 640.0).clamp(0.6, 1.0);
        // Stepper band.
        let band = Rect::from_min_size(full.min, vec2(full.width(), 112.0 * k));
        ui.painter().rect_filled(band, 0.0, theme::card());
        ui.painter().line_segment(
            [band.left_bottom(), band.right_bottom()],
            egui::Stroke::new(1.0_f32, theme::border()),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(band.shrink2(vec2(0.0, 8.0 * k))));
        let reachable = match (&self.input, &self.output) {
            (None, _) => 0,
            (Some(_), None) => 1,
            (Some(_), Some(_)) if self.step >= 3 => self.step,
            _ => 2,
        };
        let running = self.job.is_some();
        let convert_idx = self.convert_idx();
        let labels: &[&str] = if self.has_replace_step() { &STEPS_JAVA } else { &STEPS };
        if let Some(s) = widgets::stepper(
            &mut child,
            labels,
            self.step,
            if running { self.step } else { reachable.min(convert_idx) },
            k,
        ) {
            self.step = s;
        }
        let arrow = |ui: &mut Ui, x: f32, icon: Icon, id: &str| -> bool {
            let r = Rect::from_center_size(pos2(x, band.top() + 60.0 * k), Vec2::splat(28.0 * k));
            let resp = ui.interact(r, ui.id().with(id), Sense::click());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(r, theme::radius(4), theme::wash(28));
            }
            icons::paint(
                ui.painter(),
                r.shrink(6.0 * k),
                icon,
                if resp.hovered() {
                    theme::strong()
                } else {
                    theme::text_muted()
                },
            );
            resp.clicked()
        };
        let cx = band.center().x;
        // Beyond the first and last step buttons, whatever the number of steps.
        let reach = (60.0 * (labels.len() - 1) as f32 + 48.0) * k;
        if self.step > 0
            && !running
            && self.step <= convert_idx
            && arrow(ui, cx - reach, Icon::ChevronLeft, "back")
        {
            self.step -= 1;
        }
        if self.step < reachable.min(convert_idx)
            && !running
            && arrow(ui, cx + reach, Icon::ChevronRight, "fwd")
        {
            self.step += 1;
        }
        // Conversion is new and has not been tried in the games yet. The banner sits in its
        // own strip under the stepper, so it never covers the step's content.
        let text = "Early and untested: world conversion has not been verified in Minecraft. \
                    Keep a backup of your world, and expect missing or broken parts.";
        let font = theme::regular(13.0 * k);
        let invite = vec2(150.0, 26.0) * k;
        let margin = 16.0 * k;
        // The banner spans the page between equal margins; the text wraps inside it and the
        // button always sits centerd underneath, so nothing jumps while the window resizes.
        let width = (full.width() - 2.0 * margin).max(200.0);
        let galley = ui
            .painter()
            .layout(text.into(), font, theme::YELLOW, width - 28.0 * k);
        let banner = Rect::from_min_size(
            pos2(full.center().x - width / 2.0, band.bottom() + margin),
            vec2(width, galley.size().y + invite.y + 36.0 * k),
        );
        // The same margin all round: below the banner and under the content.
        let body = Rect::from_min_max(
            pos2(full.left(), banner.bottom() + margin),
            pos2(full.right(), full.bottom() - margin),
        );
        // A step taller than the window scrolls instead of being cut off.
        let need: f32 = match self.step {
            0 => 0.0,
            1 => 300.0,
            2 => 372.0,
            // (Its list gives up height to fit: at the least, the list's shortest.)
            3 if self.has_replace_step() => 470.0 - 262.0 + 60.0,
            _ if self.job.is_some() || self.result.is_some() => 420.0,
            _ => 278.0,
        };
        // A step that does not fit the window is scaled down to fit (to 55% at the most; below
        // that it scrolls). It is drawn in a layer of its own that carries the scale, so
        // everything in it, and the drop-downs hung from it, shrink together.
        // The card is as tall as it came out last time, if that is more than the table says.
        let real = self.real_height[self.step.min(5)];
        let need = if need > 0.0 { need.max(real) } else { need };
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("converter-real-height"), real));
        let scale = if need > 0.0 {
            (body.height() / need).min(body.width() / 720.0).clamp(0.7, 1.0)
        } else {
            1.0
        };
        let transform = egui::emath::TSTransform::new(body.min.to_vec2(), scale);
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("converter-step"));
        let ctx = ui.ctx().clone();
        ctx.set_transform_layer(layer, transform);
        widgets::set_page_transform(&ctx, Some(transform));
        let inner = Rect::from_min_size(egui::Pos2::ZERO, body.size() / scale);
        self.card_w = (inner.width() - 48.0).clamp(680.0, 980.0);
        self.room_h = inner.height();
        let mut measured: Option<(usize, f32)> = None;
        egui::Area::new(egui::Id::new("converter-step"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::Pos2::ZERO)
            .constrain(false)
            .movable(false)
            .show(&ctx, |ui| {
                ui.set_clip_rect(inner);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                child.set_clip_rect(inner);
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(&mut child, |ui| {
                        let (area, _) = ui.allocate_exact_size(
                            vec2(inner.width(), inner.height().max(need)),
                            Sense::hover(),
                        );
                        let mut page = ui.new_child(egui::UiBuilder::new().max_rect(area));
                        let step = self.step;
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
                            3 if self.has_replace_step() => self.replace_step(&mut page),
                            _ => self.convert_step(&mut page, settings),
                        }
                        // The card's own height: from where it starts (see `centered`) down.
                        let top: Option<f32> = page.ctx().data(|d| d.get_temp(egui::Id::new("converter-card-top")));
                        let from = top.unwrap_or(page.min_rect().top()).max(page.min_rect().top());
                        measured = Some((step, page.min_rect().bottom() - from));
                        page.ctx().data_mut(|d| d.remove::<f32>(egui::Id::new("converter-card-top")));
                    });
            });
        widgets::set_page_transform(&ctx, None);
        if let Some((step, height)) = measured {
            self.real_height[step.min(5)] = height;
        }
        ui.painter().rect_filled(banner, 6.0, theme::card());
        ui.painter().rect_stroke(
            banner,
            6.0,
            egui::Stroke::new(1.0_f32, theme::YELLOW),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            pos2(banner.left() + 14.0 * k, banner.top() + 12.0 * k),
            galley,
            theme::YELLOW,
        );
        let button = Rect::from_center_size(
            pos2(banner.center().x, banner.bottom() - 12.0 * k - invite.y / 2.0),
            invite,
        );
        let resp = ui
            .interact(button, ui.id().with("discord"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Join the MCToolKit Discord to report problems");
        ui.painter().rect_filled(
            button,
            theme::radius(4),
            if resp.hovered() { theme::button_hover() } else { theme::button() },
        );
        ui.painter().text(
            button.center(),
            Align2::CENTER_CENTER,
            "Report on Discord",
            theme::regular(13.0 * k),
            theme::text(),
        );
        if resp.clicked() {
            widgets::open_discord(ui);
        }
        request
    }

    /// Starts scanning the input world against the chosen Java version, unless that scan is
    /// already done or running.
    fn ensure_scan(&mut self, ctx: &egui::Context) {
        let Some((src, _)) = self.input.clone() else {
            return;
        };
        // 0: Java to Java, 1: Java to Bedrock, 2: Bedrock to Java.
        // 3: Java to a console game.
        // 4: Bedrock to Bedrock and 5: Bedrock to a console game, which are scanned in the Java
        // world the conversion goes through.
        let mode: u8 = match (self.input.as_ref().map(|i| i.1), self.output) {
            (Some(UiEdition::Bedrock), Some(UiEdition::Bedrock)) => 4,
            (Some(UiEdition::Bedrock), Some(UiEdition::Xbox360 | UiEdition::Ps3 | UiEdition::WiiU)) => 5,
            (Some(UiEdition::Bedrock), _) => 2,
            (_, Some(UiEdition::Bedrock)) => 1,
            (_, Some(UiEdition::Xbox360 | UiEdition::Ps3 | UiEdition::WiiU)) => 3,
            _ => 0,
        };
        let key = (src.clone(), self.java_version + 1000 * BEDROCK_INDEX.load(std::sync::atomic::Ordering::Relaxed), mode);
        if self.scan_for.as_ref() == Some(&key) {
            return;
        }
        self.scan_for = Some(key);
        self.scanned = None;
        self.scan_error = None;
        // Bedrock worlds are scanned against the newest Java; an older one is reached from it.
        let version = if matches!(mode, 2 | 3 | 5) { Self::java_versions()[0] } else { self.java_version_name() };
        self.scan_job = Some(Job::spawn("Scanning", ctx, move || {
            let bedrock = || mctk_registry::builtin::bedrock(Self::bedrock_version()).ok_or("unknown Bedrock version");
            // A Bedrock world being read is read as the newest version of the game.
            let newest_bedrock = || {
                mctk_registry::builtin::bedrock(mctk_registry::builtin::bedrock_versions()[0]).ok_or("unknown Bedrock version")
            };
            let java = || mctk_registry::builtin::java(version).ok_or("unknown output version");
            // The Java world a Bedrock world goes through on its way to a Bedrock version or a
            // console game: made here, only to be scanned.
            let through_java = || -> Result<mctk_replace::Scan, String> {
                let stage = std::env::temp_dir().join(format!("mctk-scan-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&stage);
                let made = bedrock_to_java_job(
                    &src,
                    &stage,
                    &ReplacementMap::default(),
                    &ConvertOptions::default(),
                    &Progress::default(),
                );
                let scan = made.and_then(|_| mctk_replace::Scan::java_world(&stage).map_err(|e| e.to_string()));
                let _ = std::fs::remove_dir_all(&stage);
                scan
            };
            // The registry is what the target can hold, in the source's ids: for Bedrock output
            // the Java-side view of the game, for Bedrock input the Bedrock-side view of Java.
            let (registry, scan, renames) = match mode {
                1 => (
                    mctk_convert::bedrock_view(&bedrock()?),
                    mctk_replace::Scan::java_world(&src).map_err(|e| e.to_string())?,
                    mctk_registry::Renames::java_blocks(),
                ),
                2 => (
                    mctk_convert::java_view(&java()?, &newest_bedrock()?),
                    mctk_replace::Scan::bedrock_world(&src).map_err(|e| e.to_string())?,
                    mctk_registry::Renames::default(),
                ),
                4 => (
                    mctk_convert::bedrock_view(&bedrock()?),
                    through_java()?,
                    mctk_registry::Renames::java_blocks(),
                ),
                5 => (
                    mctk_convert::legacy_registry(&java()?),
                    through_java()?,
                    mctk_registry::Renames::java_blocks(),
                ),
                // What a console game can hold, in Java's names (the console game's own source
                // says which blocks exist).
                3 => (
                    mctk_convert::legacy_registry(&java()?),
                    mctk_replace::Scan::java_world(&src).map_err(|e| e.to_string())?,
                    mctk_registry::Renames::java_blocks(),
                ),
                _ => (
                    java()?,
                    mctk_replace::Scan::java_world(&src).map_err(|e| e.to_string())?,
                    mctk_registry::Renames::java_blocks(),
                ),
            };
            let plan = Plan::build(&scan, &registry, &renames);
            Ok(Scanned { plan, registry })
        }));
    }

    /// The REPLACE step: every block the output version cannot hold, each with a suggested
    /// replacement the user can change.
    fn replace_step(&mut self, ui: &mut Ui) {
        let card = vec2(self.card_w, 470.0);
        let rect = centered(ui, card);
        let version: String = if self.output == Some(UiEdition::Bedrock) {
            format!("Bedrock {}", Self::bedrock_version())
        } else if matches!(self.output, Some(UiEdition::Xbox360 | UiEdition::Ps3 | UiEdition::WiiU)) {
            "the console game".to_string()
        } else if self.input.as_ref().map(|i| i.1) == Some(UiEdition::Bedrock) {
            Self::java_versions()[0].to_string()
        } else {
            self.java_version_name().to_string()
        };
        let ctx = ui.ctx().clone();
        let mut scanned = self.scanned.take();
        let (mut next, mut retry) = (false, false);
        let drawn = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Blocks to Replace").size(20.0).color(theme::strong()))
                });
                ui.add_space(8.0);
                match (&mut scanned, &self.scan_error) {
                    (None, Some(err)) => {
                        ui.label(RichText::new(format!("Scan failed: {err}")).color(theme::YELLOW));
                        ui.add_space(12.0);
                        retry = widgets::big_button(ui, "Try Again", vec2(160.0, 40.0)).clicked();
                    }
                    (None, None) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new(format!("Scanning the world for blocks {version} cannot hold\u{2026}"))
                                    .color(theme::text_muted()),
                            );
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                    }
                    (Some(s), _) if s.plan.is_empty() => {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(format!("Every block in this world exists in {version}."))
                                    .size(15.0)
                                    .color(theme::text()),
                            );
                            ui.label(RichText::new("Nothing needs to be replaced.").color(theme::text_muted()));
                            ui.add_space(40.0);
                            next = widgets::primary(ui, "Next", vec2(160.0, 50.0), true).clicked();
                        });
                    }
                    (Some(s), _) => {
                        let total: u64 = s.plan.groups.iter().map(|g| g.total()).sum();
                        ui.label(
                            RichText::new(format!(
                                "{} kinds of blocks ({} blocks) don't exist in {version} as they are. \
                                 Each has a suggested replacement; change any you like.",
                                s.plan.groups.len(),
                                fmt_int(total as usize)
                            ))
                            .size(14.5)
                            .color(theme::text_muted()),
                        );
                        ui.add_space(8.0);
                        let Scanned { plan, registry } = s;
                        // The list takes exactly the height the rest of the card leaves in the
                        // window (as the card came out last time; it scrolls), so the card fits.
                        let rest = match self.replace_list_h {
                            Some(last) if self.replace_card_h > last => self.replace_card_h - last,
                            _ => 470.0 - 262.0,
                        };
                        let list_h = (self.room_h - rest - 12.0).clamp(60.0, 520.0).floor();
                        self.replace_list_h = Some(list_h);
                        let extra = (card.x - 680.0).max(0.0);
                        egui::ScrollArea::vertical()
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                            .min_scrolled_height(list_h)
                            .max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                            for i in 0..plan.groups.len() {
                                ui.horizontal(|ui| {
                                    let g = &plan.groups[i];
                                    let name = g.name.trim_start_matches("minecraft:").to_string();
                                    let count = fmt_int(g.total() as usize);
                                    let font = theme::regular(15.0);
                                    let shown = widgets::elide(ui, &name, &font, 215.0 + extra * 0.5);
                                    ui.add_sized(
                                        [80.0, 28.0],
                                        egui::Label::new(RichText::new(count).font(font.clone()).color(theme::text_muted())),
                                    );
                                    ui.add_sized(
                                        [220.0 + extra * 0.5, 28.0],
                                        egui::Label::new(RichText::new(shown).font(font).color(theme::text())),
                                    );
                                    ui.label(RichText::new("\u{2192}").color(theme::text_dim()));
                                    let label = match &g.choice {
                                        Choice::Suggested => format!("Suggested: {}", g.suggestion_text().trim_start_matches("minecraft:")),
                                        Choice::Air => "Remove".into(),
                                        Choice::Keep => "Keep as is".into(),
                                        Choice::Block(b) => b.trim_start_matches("minecraft:").to_string(),
                                    };
                                    let suggested_text = g.suggestion_text().trim_start_matches("minecraft:").to_string();
                                    let label = widgets::elide(ui, &label, &theme::regular(14.0), 190.0 + extra * 0.45);
                                    let resp = widgets::button(ui, None, &label, 230.0 + extra * 0.45, true);
                                    let popup = ui.id().with(("replace-choice", i));
                                    if resp.clicked() {
                                        self.search.clear();
                                        ui.memory_mut(|m| m.toggle_popup(popup));
                                    }
                                    let mut chosen: Option<Choice> = None;
                                    // The list is a layer of its own: it takes the page's scale.
                                    if let Some(t) = widgets::page_transform(ui.ctx()) {
                                        ui.ctx().set_transform_layer(
                                            egui::LayerId::new(egui::Order::Foreground, popup),
                                            t,
                                        );
                                    }
                                    egui::popup_below_widget(
                                        ui,
                                        popup,
                                        &resp,
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                        |ui| {
                                            ui.set_min_width(250.0);
                                            let cur = &plan.groups[i].choice;
                                            if ui
                                                .selectable_label(*cur == Choice::Suggested, format!("Suggested: {suggested_text}"))
                                                .clicked()
                                            {
                                                chosen = Some(Choice::Suggested);
                                            }
                                            if ui.selectable_label(*cur == Choice::Air, "Remove (leave air)").clicked() {
                                                chosen = Some(Choice::Air);
                                            }
                                            if ui
                                                .selectable_label(*cur == Choice::Keep, "Keep as is (may not load)")
                                                .clicked()
                                            {
                                                chosen = Some(Choice::Keep);
                                            }
                                            ui.separator();
                                            ui.label(RichText::new("Use another block").size(12.0).color(theme::text_muted()));
                                            ui.add(
                                                egui::TextEdit::singleline(&mut self.search)
                                                    .hint_text("search blocks")
                                                    .desired_width(240.0),
                                            );
                                            for c in mctk_replace::candidates(registry, &self.search, 8) {
                                                let short = c.trim_start_matches("minecraft:").to_string();
                                                if ui.selectable_label(*cur == Choice::Block(c.clone()), short).clicked() {
                                                    chosen = Some(Choice::Block(c));
                                                }
                                            }
                                        },
                                    );
                                    if let Some(c) = chosen {
                                        plan.groups[i].choice = c;
                                        ui.memory_mut(|m| m.close_popup());
                                    }
                                    let _ = mctk_replace::reason_label;
                                });
                            }
                        });
                        ui.add_space(8.0);
                        let removed = plan.blocks_removed();
                        let line = if removed > 0 {
                            format!("{} blocks will be removed.", fmt_int(removed as usize))
                        } else {
                            "No blocks will be removed.".to_string()
                        };
                        ui.label(RichText::new(line).size(14.0).color(theme::text_muted()));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if widgets::button(ui, None, "Accept All Suggestions", 0.0, plan.changed() > 0).clicked() {
                                plan.reset();
                            }
                            if widgets::button(ui, None, "Save Choices\u{2026}", 0.0, true).clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Replacement choices", &["tsv"])
                                    .set_file_name("replacements.tsv")
                                    .save_file()
                                {
                                    self.status = match std::fs::write(&path, plan.to_map().to_text()) {
                                        Ok(()) => format!("Saved choices to {}", path.display()),
                                        Err(e) => format!("Could not save: {e}"),
                                    };
                                }
                            }
                            if widgets::button(ui, None, "Load Choices\u{2026}", 0.0, true).clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("Replacement choices", &["tsv"]).pick_file() {
                                    self.status = match std::fs::read_to_string(&path)
                                        .map_err(|e| e.to_string())
                                        .and_then(|t| ReplacementMap::from_text(&t).map_err(|e| e.to_string()))
                                    {
                                        Ok(map) => {
                                            plan.apply_map(&map);
                                            format!("Loaded choices from {}", path.display())
                                        }
                                        Err(e) => format!("Could not load: {e}"),
                                    };
                                }
                            }
                        });
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| {
                            next = widgets::primary(ui, "Next", vec2(160.0, 46.0), true).clicked();
                        });
                    }
                }
            });
        });
        // How tall the card came out (its list's height is fitted to the window from it).
        self.replace_card_h = drawn.response.rect.height();
        self.scanned = scanned;
        if retry {
            self.scan_for = None;
        }
        if next {
            self.step = 4;
        }
    }

    fn output_step(&mut self, ui: &mut Ui) {
        let card = vec2(self.card_w, 300.0);
        let rect = centered(ui, card);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Select an Output Version")
                            .size(20.0)
                            .color(theme::strong()),
                    )
                });
                ui.add_space(12.0);
                // The tiles share the card's width (wider in a wide window), centered.
                let inner = card.x - 40.0;
                let n = UiEdition::ALL.len() as f32;
                let tile_w = ((inner - 10.0 * (n - 1.0)) / n).clamp(120.0, 170.0).floor();
                let row_w = tile_w * n + 10.0 * (n - 1.0);
                ui.horizontal(|ui| {
                    ui.add_space(((inner - row_w) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 10.0;
                    for e in UiEdition::ALL {
                        let ok = self.supported(e);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let resp = widgets::tile(
                                ui,
                                e.icon(),
                                e.label(),
                                vec2(tile_w, 120.0),
                                self.output == Some(e),
                                ok,
                                true,
                            );
                            let resp = if ok {
                                resp
                            } else {
                                resp.on_hover_text("Not available in MCToolKit yet")
                            };
                            if resp.clicked() {
                                self.output = Some(e);
                            }
                            let version = self.version_label(e);
                            // The strip under a tile picks the output version: clicking it chooses
                            // the edition too and lists the versions it can be written as.
                            let (r, vresp) = ui.allocate_exact_size(
                                vec2(tile_w, 30.0),
                                if ok { Sense::click() } else { Sense::hover() },
                            );
                            let popup = ui.id().with(("version-popup", e as u8));
                            let open = ok && widgets::dropdown_open(ui, popup);
                            // The strip takes the color of its tile: green once the edition is chosen.
                            let chosen = self.output == Some(e);
                            let (base, lit, tint) = if chosen {
                                // A shade darker than the tile, so the strip is told apart from it.
                                let strip = theme::accent_dark();
                                (strip, theme::accent_hover(), (theme::button_hover(), strip))
                            } else {
                                (
                                    theme::tile(),
                                    theme::button_hover(),
                                    (theme::button_hover(), theme::wash(14)),
                                )
                            };
                            let edge = egui::Stroke::new(1.0_f32, theme::border());
                            ui.painter().rect_filled(
                                r,
                                egui::CornerRadius {
                                    nw: 0,
                                    ne: 0,
                                    sw: if open { 0 } else { 5 },
                                    se: if open { 0 } else { 5 },
                                },
                                if ok && vresp.hovered() { lit } else { base },
                            );
                            // The same outline as the tile, with the seam between them painted
                            // over so the two read as one piece.
                            ui.painter().rect_stroke(
                                r,
                                egui::CornerRadius {
                                    nw: 0,
                                    ne: 0,
                                    sw: if open { 0 } else { 5 },
                                    se: if open { 0 } else { 5 },
                                },
                                edge,
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().rect_filled(
                                Rect::from_min_max(
                                    pos2(r.left() + 1.0, r.top() - 1.0),
                                    pos2(r.right() - 1.0, r.top() + 1.0),
                                ),
                                0.0,
                                base,
                            );
                            if open {
                                // Joined to the list below: no line between them.
                                ui.painter().rect_filled(
                                    Rect::from_min_max(
                                        pos2(r.left() + 1.0, r.bottom() - 1.0),
                                        pos2(r.right() - 1.0, r.bottom()),
                                    ),
                                    0.0,
                                    base,
                                );
                            }
                            let font = theme::regular(14.5);
                            let version = widgets::elide(ui, &version, &font, r.width() - 34.0);
                            ui.painter().text(
                                r.left_center() + vec2(7.0, 0.0),
                                Align2::LEFT_CENTER,
                                version,
                                font,
                                if ok { theme::text() } else { theme::text_dim() },
                            );
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    r.right_center() - vec2(12.0, 0.0),
                                    Vec2::splat(9.0),
                                ),
                                if open { Icon::ChevronUp } else { Icon::ChevronDown },
                                if chosen { theme::strong() } else { theme::text_muted() },
                            );
                            if ok {
                                if vresp.clicked() {
                                    self.output = Some(e);
                                    ui.memory_mut(|m| m.toggle_popup(popup));
                                }
                                widgets::dropdown(
                                    ui,
                                    popup,
                                    &vresp,
                                    theme::tile(),
                                    Some(edge),
                                    |ui, _| {
                                        match e {
                                            UiEdition::Java => {
                                                let count = Self::java_versions().len();
                                                for (i, v) in Self::java_versions().iter().enumerate() {
                                                    let last = i + 1 == count;
                                                    if widgets::dropdown_item(ui, self.java_version == i, last, tint, v).clicked() {
                                                        self.java_version = i;
                                                    }
                                                }
                                            }
                                            UiEdition::Bedrock => {
                                                let count = mctk_registry::builtin::bedrock_versions().len();
                                                for (i, v) in mctk_registry::builtin::bedrock_versions().iter().enumerate() {
                                                    let label = if i == 0 { format!("{v} (Latest)") } else { v.to_string() };
                                                    let chosen = BEDROCK_INDEX.load(std::sync::atomic::Ordering::Relaxed) == i;
                                                    if widgets::dropdown_item(ui, chosen, i + 1 == count, tint, &label).clicked() {
                                                        BEDROCK_INDEX.store(i, std::sync::atomic::Ordering::Relaxed);
                                                    }
                                                }
                                            }
                                            _ => {
                                                widgets::dropdown_item(ui, true, true, tint, Self::CONSOLE_VERSION);
                                            }
                                        }
                                    },
                                );
                            }                        });
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
        let card = vec2(self.card_w, 372.0);
        let rect = centered(ui, card);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Conversion Options")
                            .size(20.0)
                            .color(theme::strong()),
                    )
                });
                ui.add_space(14.0);
                // Two columns that share the card's width (wider in a wide window), centered.
                let inner = card.x - 40.0;
                let col = ((inner - 26.0) / 2.0).clamp(310.0, 430.0).floor();
                ui.horizontal_top(|ui| {
                    ui.add_space(((inner - (2.0 * col + 26.0)) / 2.0).max(0.0));
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        ui.horizontal(|ui| {
                            widgets::toggle(ui, &mut self.dims[0], "Overworld", true);
                            widgets::toggle(ui, &mut self.dims[1], "Nether", true);
                            widgets::toggle(ui, &mut self.dims[2], "The End", true);
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            widgets::toggle(ui, &mut self.tiles, "Tile Entities", true)
                                .on_hover_text("Chests, signs, furnaces and the other blocks that hold data. Off leaves them empty");
                            widgets::toggle(ui, &mut self.mobs, "Entities", true)
                                .on_hover_text("Mobs, paintings, items on the ground and the like. Off leaves them out");
                            widgets::toggle(ui, &mut self.ticks, "Ticks", true)
                                .on_hover_text("Keep scheduled block and fluid ticks (Java to Java; other targets work them out again)");
                        });
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Game Mode")
                                .size(15.0)
                                .color(theme::text_muted()),
                        );
                        widgets::segmented(
                            ui,
                            &mut self.game_mode,
                            &["No Change", "Survival", "Creative", "Adventure"],
                            col,
                            true,
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Difficulty")
                                .size(15.0)
                                .color(theme::text_muted()),
                        );
                        widgets::segmented(
                            ui,
                            &mut self.difficulty,
                            &["No Change", "Peaceful", "Easy", "Normal", "Hard"],
                            col,
                            true,
                        );
                    });
                    let x = ui.cursor().left();
                    ui.painter().line_segment(
                        [
                            pos2(x + 2.0, ui.cursor().top()),
                            pos2(x + 2.0, ui.cursor().top() + 160.0),
                        ],
                        egui::Stroke::new(1.0_f32, theme::border()),
                    );
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        widgets::toggle(
                            ui,
                            &mut self.skip_partial,
                            "Skip Partially-Generated Chunks",
                            true,
                        )
                        .on_hover_text("Leave out chunks the game had not finished generating");
                        ui.add_space(6.0);
                        widgets::toggle(ui, &mut self.older_chunks, "Allow Older Chunk Versions", true)
                            .on_hover_text("Off leaves out chunks saved by a version older than the world's own (Java sources)");
                        ui.add_space(10.0);
                        ui.label(RichText::new("Depth Fixer").size(15.0).color(theme::text_muted()))
                            .on_hover_text("For targets whose world starts at y 0 (Java before 1.18, Bedrock before 1.18, consoles): what to do with the overworld below it");
                        widgets::segmented(ui, &mut self.depth, &["Clip + Bedrock", "Clip", "Raise"], col - 20.0, true);
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("New World Center")
                                .size(15.0)
                                .color(theme::text_muted()),
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
        if std::mem::take(&mut self.dev_start) && self.job.is_none() {
            let ctx = ui.ctx().clone();
            self.start(&ctx, settings);
        }
        let running = self.job.is_some();
        let done = matches!(self.result, Some(Ok(_)));
        let tall = running || self.result.is_some();
        // The Convert step shows the conversion; once it is done, Next goes to the Save step,
        // which offers Save World and Start Over.
        let save_idx = self.convert_idx() + 1;
        let save_step = self.step >= save_idx;
        let card = vec2(self.card_w, if tall { 420.0 } else { 278.0 });
        let rect = centered(ui, card);
        let ctx = ui.ctx().clone();
        let drawn = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                // Source -> target.
                let (r, _) = ui.allocate_exact_size(vec2(card.x - 40.0, 92.0), Sense::hover());
                let (src, dst) = (self.input.as_ref().map(|i| i.1), self.output);
                let p = ui.painter();
                if let (Some(s), Some(d)) = (src, dst) {
                    let c = r.center();
                    icons::paint(
                        p,
                        Rect::from_center_size(c - vec2(80.0, 20.0), Vec2::splat(40.0)),
                        s.icon(),
                        theme::text(),
                    );
                    p.text(
                        c - vec2(80.0, -16.0),
                        Align2::CENTER_CENTER,
                        s.label(),
                        theme::regular(14.0),
                        theme::text(),
                    );
                    icons::paint(
                        p,
                        Rect::from_center_size(c - vec2(0.0, 20.0), Vec2::splat(64.0)),
                        Icon::ArrowRight,
                        theme::text(),
                    );
                    icons::paint(
                        p,
                        Rect::from_center_size(c + vec2(80.0, -20.0), Vec2::splat(40.0)),
                        d.icon(),
                        theme::text(),
                    );
                    p.text(
                        c + vec2(80.0, 16.0),
                        Align2::CENTER_CENTER,
                        d.label(),
                        theme::regular(14.0),
                        theme::text(),
                    );
                    let v = self.version_label(d);
                    p.text(
                        c + vec2(80.0, 33.0),
                        Align2::CENTER_CENTER,
                        v,
                        theme::regular(13.0),
                        theme::text_dim(),
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
                    (_, true) if total > 0 => format!("Converted {} of {} Chunks", fmt_int(n), fmt_int(total)),
                    (_, true) => "Reading the World...".into(),
                    (Some(Ok(c)), _) => format!(
                        "Conversion Successful! {} Chunks Converted",
                        fmt_int(c.lines().next().unwrap_or("").parse().unwrap_or(n))
                    ),
                    (Some(Err(e)), _) => format!("Conversion Failed: {e}"),
                    (None, false) => "Ready to Convert".into(),
                };
                // Lines after the count: what could not be carried over (Java to Java).
                let notes: Vec<String> = match &self.result {
                    Some(Ok(c)) => c.lines().skip(1).map(String::from).collect(),
                    _ => Vec::new(),
                };
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(headline).size(15.5).color(theme::text()));
                });
                // What was left out: a list in the height the rest of the card leaves in the
                // window (as the card came out last time), scrolling when longer.
                if !notes.is_empty() {
                    let rest = match self.notes_h {
                        Some(last) if self.convert_card_h > last => self.convert_card_h - last,
                        _ => 420.0,
                    };
                    let want = notes.len() as f32 * 20.0;
                    let notes_h = (self.room_h - rest - 12.0).clamp(40.0, want.max(40.0)).floor();
                    self.notes_h = Some(notes_h);
                    egui::ScrollArea::vertical()
                        .id_salt("convert-notes")
                        .max_height(notes_h)
                        .min_scrolled_height(notes_h.min(want))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for line in &notes {
                                    ui.label(RichText::new(line).size(13.5).color(theme::text_muted()));
                                }
                            });
                        });
                } else {
                    self.notes_h = None;
                }
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
                // (Once done, in a short window, the stats give way so the buttons fit.)
                let short = self.room_h < 470.0;
                if tall && !(done && short) {
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
                    } else if done && !save_step {
                        // The conversion is finished; the Save step is next.
                        if widgets::primary(ui, "Next", vec2(160.0, 50.0), true).clicked() {
                            self.step = save_idx;
                        }
                    } else if done {
                        ui.horizontal(|ui| {
                            ui.add_space((card.x - 40.0 - 420.0) / 2.0);
                            if widgets::big_button(ui, "Start Over", vec2(200.0, 50.0)).clicked() {
                                self.reset();
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
        // How tall the card came out (its list of what was left out is fitted from it).
        self.convert_card_h = drawn.response.rect.height();
    }

    /// Every chunk of the world's dimension `dim` the conversion will go through, read in the
    /// background the first time it is asked for (`None` until then, and when the output is
    /// moved to a new center, where the converted chunks are not where they were).
    fn footprint(&mut self, ctx: &egui::Context, dim: &str) -> Option<Arc<Vec<(i32, i32)>>> {
        if self.center_spawn || self.center != (0.0, 0.0) || dim.is_empty() {
            return None;
        }
        let (path, edition) = self.input.clone()?;
        let key = format!("{}|{dim}", path.display());
        if let Some(Footprint::Reading(job)) = self.footprints.get(&key) {
            if let Some(r) = job.take() {
                let done = match r {
                    Ok(v) => Footprint::Ready(Arc::new(v)),
                    Err(_) => Footprint::Failed,
                };
                self.footprints.insert(key.clone(), done);
            }
        }
        match self.footprints.get(&key) {
            Some(Footprint::Ready(v)) => return Some(v.clone()),
            Some(_) => return None,
            None => {}
        }
        let java_dim = match dim {
            "minecraft:the_nether" => -1,
            "minecraft:the_end" => 1,
            _ => 0,
        };
        let job = Job::spawn("Reading the world's chunks", ctx, move || {
            let chunks = match edition {
                UiEdition::Java => mctk_prune::chunk_times_java(&path, java_dim, false),
                UiEdition::Bedrock => mctk_prune::chunk_times_bedrock(&path, if java_dim == 0 { 0 } else if java_dim == -1 { 1 } else { 2 }),
                _ => mctk_prune::chunk_times_console(&path, java_dim, false),
            }
            .map_err(|e| e.to_string())?;
            Ok(chunks.into_keys().collect())
        });
        self.footprints.insert(key, Footprint::Reading(job));
        None
    }

    fn stats(&mut self, ui: &mut Ui, width: f32, running: bool) {
        // With room, a bigger picture of the conversion.
        let side = if self.room_h >= 560.0 { 160.0 } else { 96.0 };
        let (r, _) = ui.allocate_exact_size(vec2(width, side + 26.0), Sense::hover());
        let dim_now = self.progress.as_ref().map(|p| p.dimension.lock().expect("progress lock").clone()).unwrap_or_default();
        let all = self.footprint(ui.ctx(), &dim_now);
        let p = ui.painter();
        p.rect_filled(r, theme::radius(4), theme::bg());
        // The world being converted (the dimension now), filling in as its chunks are done.
        let pv = Rect::from_min_size(r.min + vec2(16.0, 13.0), Vec2::splat(side));
        let frame = Color32::from_rgb(0x0F, 0x11, 0x16);
        p.rect_filled(pv, theme::radius(3), frame);
        let Some(prog) = self.progress.clone() else {
            return;
        };
        {
            let pts = prog.converted.lock().expect("progress lock");
            let start = prog.dimension_start.load(Ordering::Relaxed).min(pts.len());
            let colors = crate::chunk_preview::Colors {
                frame,
                chunk: theme::accent(),
                fresh: Color32::from_rgb(0xE6, 0x9A, 0x2E),
                todo: Color32::from_rgb(0x30, 0x35, 0x3F),
            };
            // A picture, a pixel a chunk, scaled up with sharp edges (so it stays whole however
            // the page is scaled).
            let img = crate::chunk_preview::image(&pts[start..], all.as_deref().map(|v| v.as_slice()), running, &colors, 256);
            drop(pts);
            let tex = match &mut self.preview_tex {
                Some(t) => {
                    t.set(img, egui::TextureOptions::NEAREST);
                    t.clone()
                }
                None => {
                    let t = ui.ctx().load_texture("conversion-picture", img, egui::TextureOptions::NEAREST);
                    self.preview_tex = Some(t.clone());
                    t
                }
            };
            let inner = pv.shrink(3.0);
            ui.painter().image(tex.id(), inner, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        let p = ui.painter();        let elapsed = if running {
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
            memory_mb().map_or("—".into(), |m| format!("{m}MB"))
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
                // Java and Bedrock worlds are lit by the game as they load (Java's chunks are
                // marked for it); a console save's light is worked out in the conversion.
                (
                    "Lighting:",
                    match self.output {
                        Some(UiEdition::Java | UiEdition::Bedrock) => "By the game".into(),
                        Some(_) => "Worked out".into(),
                        None => String::new(),
                    },
                ),
            ],
        ];
        for (ci, col) in cols.iter().enumerate() {
            for (ri, (k, v)) in col.iter().enumerate() {
                let pos = pos2(
                    r.left() + 132.0 + ci as f32 * 166.0,
                    r.top() + 27.0 + ri as f32 * 34.0,
                );
                let kg = p.layout_no_wrap(k.to_string(), theme::regular(14.0), theme::text_muted());
                let w = kg.size().x;
                p.galley(pos - vec2(0.0, 8.0), kg, theme::text_muted());
                p.text(
                    pos + vec2(w + 4.0, 0.0),
                    Align2::LEFT_CENTER,
                    v,
                    theme::bold(13.0),
                    theme::strong(),
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
        let mut dialog = rfd::FileDialog::new().set_title("Save the converted world into…");
        if std::path::Path::new(default_dir).is_dir() {
            dialog = dialog.set_directory(default_dir);
        }
        let Some(dest_parent) = dialog.pick_folder() else {
            return;
        };
        let name = self
            .input
            .as_ref()
            .map_or("Converted World".into(), |i| world_name(&i.0));
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

/// What a world is called: the folder for Java and Bedrock worlds (`.mcworld` files without
/// the extension), and for the console saves whose file is named alike in every save (a PS3
/// `GAMEDATA`, an Xbox 360 `savegame.dat`) the folder the file sits in.
fn world_name(path: &std::path::Path) -> String {
    let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let alike = file == "GAMEDATA" || file.eq_ignore_ascii_case("savegame.dat");
    let named = if alike && path.is_file() {
        path.parent().and_then(|p| p.file_name())
    } else if path.is_file() {
        path.file_stem()
    } else {
        path.file_name()
    };
    named
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Converted World".into())
}

/// Converts `src` to `out_ed`, going through a temporary Java world (in the newest version, next to
/// the output) wherever no converter goes straight there: a console save or a Bedrock world to an
/// older Java version, a console save to Bedrock, a Bedrock world to a Bedrock world. The user
/// picks the edition and version; the steps in between are not their business.
#[allow(clippy::too_many_arguments)]
fn route_job(
    ed: UiEdition,
    out_ed: UiEdition,
    src: &std::path::Path,
    target: &std::path::Path,
    java_version: &str,
    replacements: &ReplacementMap,
    options: &ConvertOptions,
    p: &Progress,
) -> Result<String, String> {
    let newest = mctk_registry::builtin::java_versions()[0];
    let none = ReplacementMap::default();
    // What the first leg's output is passed on with: the offset was applied there already.
    let pass = ConvertOptions { offset: (0, 0), ..options.clone() };
    let stage = target.with_extension("java-stage");
    let console = !matches!(ed, UiEdition::Java | UiEdition::Bedrock);
    let result = (|| -> Result<String, String> {
        // A console save as the output: a Java world in the newest version first, then the
        // console writer. The file is named as the game names it.
        if let UiEdition::Xbox360 | UiEdition::Ps3 | UiEdition::WiiU = out_ed {
            let platform = match out_ed {
                UiEdition::Xbox360 => mctk_console::Platform::Xbox360,
                UiEdition::WiiU => mctk_console::Platform::WiiU,
                _ => mctk_console::Platform::Ps3,
            };
            let reg = mctk_registry::builtin::java(newest).ok_or("unknown Java version")?;
            let last = match ed {
                UiEdition::Java => {
                    java_to_java_job(src, &stage, newest, replacements, options, p)?;
                    options.clone()
                }
                UiEdition::Bedrock => {
                    bedrock_to_java_job(src, &stage, &none, options, p)?;
                    pass.clone()
                }
                _ => {
                    let save = mctk_console::ConsoleSave::open(src).map_err(|e| e.to_string())?;
                    mctk_convert::console_to_modern(&save, &stage, &reg, options, p).map_err(|e| e.to_string())?;
                    pass.clone()
                }
            };
            // A Java world's choices were applied on the way to the Java stage; a Bedrock world's
            // were made against that stage, so the console writer applies them.
            let choices = if ed == UiEdition::Bedrock { replacements } else { &none };
            std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
            let file = target.join(match out_ed {
                UiEdition::Xbox360 => "savegame.dat".to_string(),
                UiEdition::WiiU => format!("{}.wii", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "World".into())),
                _ => "GAMEDATA".to_string(),
            });
            return java_to_console_job(&stage, &file, platform, choices, &last, p);
        }
        match (ed, out_ed) {
            (UiEdition::Java, UiEdition::Java) => java_to_java_job(src, target, java_version, replacements, options, p),
            (UiEdition::Java, UiEdition::Bedrock) => java_to_bedrock_job(src, target, replacements, options, p),
            (UiEdition::Bedrock, UiEdition::Java) if java_version == newest => bedrock_to_java_job(src, target, replacements, options, p),
            (UiEdition::Bedrock, UiEdition::Java) => {
                bedrock_to_java_job(src, &stage, replacements, options, p)?;
                java_to_java_job(&stage, target, java_version, &none, &pass, p)
            }
            // The choices were made against the Java world in between (see `ensure_scan`).
            (UiEdition::Bedrock, UiEdition::Bedrock) => {
                bedrock_to_java_job(src, &stage, &none, options, p)?;
                java_to_bedrock_job(&stage, target, replacements, &pass, p)
            }
            (_, UiEdition::Java | UiEdition::Bedrock) if console => {
                let save = mctk_console::ConsoleSave::open(src).map_err(|e| e.to_string())?;
                let reg = mctk_registry::builtin::java(newest).ok_or("unknown Java version")?;
                // A console world to the newest Java is the first leg alone.
                let first = if out_ed == UiEdition::Java && java_version == newest { target } else { &stage };
                let r = mctk_convert::console_to_modern(&save, first, &reg, options, p).map_err(|e| e.to_string())?;
                let mut text = r.chunks.to_string();
                if r.players > 0 {
                    text += &format!("\n{} player(s) carried over", r.players);
                }
                if r.maps > 0 {
                    text += &format!("\n{} map(s) carried over", r.maps);
                }
                let mut notes: Vec<_> = r.notes.0.iter().collect();
                notes.sort_by(|a, b| b.1.cmp(a.1));
                for (what, n) in notes.iter().take(4) {
                    text += &format!("\n{n}\u{00D7} {what}");
                }
                match out_ed {
                    UiEdition::Bedrock => java_to_bedrock_job(&stage, target, &none, &pass, p),
                    _ if java_version == newest => Ok(text),
                    _ => java_to_java_job(&stage, target, java_version, &none, &pass, p),
                }
            }
            _ => Err("this conversion is not available".into()),
        }
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// Writes a Java world as a console save. The result is the chunk count on the first line, then a
/// line for each kind of thing that changed or was left out.
fn java_to_console_job(
    src: &std::path::Path,
    file: &std::path::Path,
    platform: mctk_console::Platform,
    replacements: &ReplacementMap,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<String, String> {
    let report = mctk_convert::java_to_console(src, file, platform, replacements, options, progress).map_err(|e| e.to_string())?;
    let mut text = report.chunks.to_string();
    let mut lines: Vec<(u64, String)> = Vec::new();
    for (what, (to, n)) in &report.replaced {
        lines.push((*n, format!("{} blocks replaced: {what} -> {to}", fmt_int(*n as usize))));
    }
    if report.outside > 0 {
        lines.push((report.outside, format!("{} chunks outside the largest console world (320 chunks across) left out", report.outside)));
    }
    for (what, n) in &report.notes.0 {
        lines.push((*n, format!("{n}\u{00D7} {what}")));
    }
    lines.push((0, "player data is not written: console player files belong to a console profile".to_string()));
    lines.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, line) in lines.iter().take(6) {
        text += &format!("\n{line}");
    }
    Ok(text)
}

/// Runs a Java to Java conversion. The result is the chunk count on the first line, then one
/// line for each thing that could not be carried over.
fn java_to_java_job(
    src: &std::path::Path,
    dst: &std::path::Path,
    version: &str,
    replacements: &ReplacementMap,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<String, String> {
    let registry = mctk_registry::builtin::java(version).ok_or("unknown output version")?;
    let target = mctk_convert::JavaTarget::for_version(registry).map_err(|e| e.to_string())?;
    let report = mctk_convert::java_to_java(src, dst, &target, replacements, options, progress)
        .map_err(|e| e.to_string())?;
    let mut text = report.chunks.to_string();
    if report.players > 0 {
        text += &format!(
            "\n{} player(s) carried over (inventory, ender chest, health, hunger, XP, spawn point, advancements, statistics)",
            report.players
        );
    }
    if report.saved_data_files > 0 {
        text += &format!("\n{} saved-data files copied (scoreboard, maps, ...)", report.saved_data_files);
    }
    if report.unsupported_chunks > 0 {
        text += &format!(
            "\n{} chunks left out: they could not be read",
            fmt_int(report.unsupported_chunks as usize)
        );
    }
    for (what, n) in report.biomes_replaced.iter().take(3) {
        text += &format!("\n{} biome cells replaced: {what}", fmt_int(*n as usize));
    }
    let mut notes: Vec<_> = report.notes.0.iter().collect();
    notes.sort_by(|a, b| b.1.cmp(a.1));
    for (what, n) in notes.iter().take(6) {
        text += &format!("\n{n}\u{00D7} {what}");
    }
    if notes.len() > 6 {
        text += &format!("\nand {} more kinds", notes.len() - 6);
    }
    Ok(text)
}

/// Runs a Bedrock to Java conversion with the user's replacement choices. The result is the chunk
/// count on the first line, then a line for each kind of thing that changed or was left out.
fn bedrock_to_java_job(
    src: &std::path::Path,
    dst: &std::path::Path,
    replacements: &ReplacementMap,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<String, String> {
    let java = mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0]).ok_or("unknown Java version")?;
    // A Bedrock world being read is read as the newest version of the game, whatever Bedrock
    // version may have been picked for an output.
    let bedrock = mctk_registry::builtin::bedrock(mctk_registry::builtin::bedrock_versions()[0]).ok_or("unknown Bedrock version")?;
    mctk_convert::set_bedrock_target(&bedrock);
    let replace = mctk_convert::JavaReplacements::new(replacements.clone(), &java, &bedrock);
    let report = mctk_convert::bedrock_to_java_replacing(src, dst, options, progress, Some(&replace))
        .map_err(|e| e.to_string())?;
    let mut text = report.chunks.values().sum::<usize>().to_string();
    let mut lines: Vec<(usize, String)> = Vec::new();
    for (what, n) in &report.replaced {
        lines.push((*n, format!("{} blocks replaced: {what}", fmt_int(*n))));
    }
    for (what, n) in &report.unmapped {
        lines.push((*n, format!("{n}\u{00D7} no Java equivalent, left as air: {what}")));
    }
    for (what, n) in &report.skipped {
        lines.push((*n, skipped_line(*n, what)));
    }
    lines.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, line) in lines.iter().take(6) {
        text += &format!("\n{line}");
    }
    if lines.len() > 6 {
        text += &format!("\nand {} more kinds", lines.len() - 6);
    }
    Ok(text)
}

/// Runs a Java to Bedrock conversion with the user's replacement choices. The result is the chunk
/// count on the first line, then a line for each kind of thing that changed or was left out.
fn java_to_bedrock_job(
    src: &std::path::Path,
    dst: &std::path::Path,
    replacements: &ReplacementMap,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<String, String> {
    let bedrock = mctk_registry::builtin::bedrock(ConverterPage::bedrock_version()).ok_or("unknown Bedrock version")?;
    mctk_convert::set_bedrock_target(&bedrock);
    let replace = mctk_convert::BedrockReplacements::new(replacements.clone(), &bedrock);
    let report = mctk_convert::java_to_bedrock_replacing(src, dst, options, progress, Some(&replace))
        .map_err(|e| e.to_string())?;
    let mut text = report.chunks.values().sum::<usize>().to_string();
    let mut lines: Vec<(usize, String)> = Vec::new();
    for (what, n) in &report.replaced {
        lines.push((*n, format!("{} blocks replaced: {what}", fmt_int(*n))));
    }
    for (what, n) in &report.unmapped {
        lines.push((*n, format!("{n}\u{00D7} no Bedrock equivalent, left as air: {what}")));
    }
    for (what, n) in &report.skipped {
        lines.push((*n, skipped_line(*n, what)));
    }
    lines.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, line) in lines.iter().take(6) {
        text += &format!("\n{line}");
    }
    if lines.len() > 6 {
        text += &format!("\nand {} more kinds", lines.len() - 6);
    }
    Ok(text)
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

/// A line of the report for something not carried over, saying why where it is on purpose:
/// unfinished chunks at the edge of what was explored are skipped (the option), and the game
/// makes them again; the rest is what the target has no form for.
fn skipped_line(n: usize, what: &str) -> String {
    match what {
        "partially generated chunks" => format!("{n} unfinished chunks at the explored edge skipped (the game generates them again)"),
        _ => format!("{n}\u{00D7} left out: {what}"),
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

#[cfg(test)]
mod tests {
    /// A console save converted to the newest Java, to an older Java and to Bedrock, each through
    /// the route the converter picks (skipped without `MCTK_TEST_CONSOLE_SAVE`).
    #[test]
    fn console_save_routes_to_every_target() {
        let Ok(save) = std::env::var("MCTK_TEST_CONSOLE_SAVE") else {
            return;
        };
        let base = std::env::temp_dir().join(format!("mctk-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let run = |name: &str, out: UiEdition, version: &str| {
            let target = base.join(name);
            let text = route_job(
                UiEdition::Xbox360,
                out,
                std::path::Path::new(&save),
                &target,
                version,
                &ReplacementMap::default(),
                &ConvertOptions::default(),
                &Progress::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!target.with_extension("java-stage").exists(), "{name}: the temporary world is removed");
            (target, text)
        };
        let newest = mctk_registry::builtin::java_versions()[0];
        let (latest, text) = run("latest", UiEdition::Java, newest);
        assert!(text.lines().next().and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n > 100), "{text}");
        assert!(latest.join("dimensions/minecraft/overworld/region").is_dir());
        let (old, _) = run("old", UiEdition::Java, "1.20.1");
        assert!(old.join("level.dat").is_file() && old.join("region").is_dir());
        let (bedrock, _) = run("bedrock", UiEdition::Bedrock, newest);
        assert!(bedrock.join("db").is_dir() && bedrock.join("level.dat").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Every input reaches Java and Bedrock (through a temporary Java world where there is no
    /// direct converter); only the console outputs, which cannot be written yet, stay off.
    #[test]
    fn every_input_reaches_java_and_bedrock() {
        let settings = Settings::default();
        for input in UiEdition::ALL {
            let mut page = ConverterPage::new(&settings);
            assert!(!page.supported(UiEdition::Java), "no input yet");
            page.set_input(PathBuf::from("world"), input);
            assert!(page.supported(UiEdition::Java) && page.supported(UiEdition::Bedrock), "{input:?}");
            assert!(page.supported(UiEdition::Xbox360) && page.supported(UiEdition::Ps3), "{input:?}");
            assert!(page.supported(UiEdition::WiiU), "{input:?}");
        }
    }

    use super::*;

    /// A finished conversion with many notes needs a taller card than the nominal one; the page
    /// notices (it measures how far the card overflows) and settles on a size that holds it.
    #[test]
    fn a_step_taller_than_its_card_settles_on_a_scale_that_holds_it() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        icons::install_textures(&ctx);
        let mut settings = Settings::default();
        let mut page = ConverterPage::new(&settings);
        page.set_input(PathBuf::from("world"), UiEdition::Xbox360);
        page.output = Some(UiEdition::Java);
        let notes = (0..9).map(|i| format!("\n{i}\u{00D7} something that could not be carried over")).collect::<String>();
        page.result = Some(Ok(format!("1273{notes}")));
        page.step = page.convert_idx() + 1;
        page.converted_for = Some(page.choice_key());
        let mut last = [0.0; 6];
        for _ in 0..8 {
            frame(&ctx, &mut page, &mut settings);
            last = page.real_height;
        }
        let before = last;
        frame(&ctx, &mut page, &mut settings);
        assert!(page.real_height[page.step] > 430.0, "{:?}", page.real_height);
        assert!(
            (page.real_height[page.step] - before[page.step]).abs() < 1.0,
            "still changing: {before:?} -> {:?}",
            page.real_height
        );
        // Looking back at the steps keeps the finished conversion; another output forgets it.
        page.converted_for = Some(page.choice_key());
        page.step = 1;
        frame(&ctx, &mut page, &mut settings);
        assert!(page.result.is_some());
        page.output = Some(UiEdition::Bedrock);
        frame(&ctx, &mut page, &mut settings);
        assert!(page.result.is_none() && page.progress.is_none());
    }
    /// Runs one frame of the page and returns the clipped shapes drawn.
    fn frame(ctx: &egui::Context, page: &mut ConverterPage, settings: &mut Settings) -> usize {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 820.0))),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                page.ui(ui, settings);
            });
        });
        out.shapes.len()
    }

    /// `MCTK_TEST_JAVA_WORLD`: a Java world newer than 1.20.1, so a 1.20.1 target has things
    /// to replace. Drives the real page: scan, render the REPLACE step, open a picker, choose a
    /// replacement, and convert with the resulting choices.
    /// A Bedrock world gets a REPLACE step on every route out of it, and converts with the
    /// choices made there. Needs `MCTK_TEST_BEDROCK_WORLD` (a Bedrock world folder or .mcworld).
    #[test]
    fn bedrock_routes_have_a_replace_step_and_convert() {
        let Ok(world) = std::env::var("MCTK_TEST_BEDROCK_WORLD") else {
            return;
        };
        let ctx = egui::Context::default();
        theme::install(&ctx);
        icons::install_textures(&ctx);
        let mut settings = Settings::default();
        let bedrock_old = mctk_registry::builtin::bedrock_versions().iter().position(|v| *v == "1.19.50.02").expect("1.19.50.02");
        for (out, index) in [(UiEdition::Bedrock, bedrock_old), (UiEdition::Xbox360, 0)] {
            let mut page = ConverterPage::new(&settings);
            page.set_input(PathBuf::from(&world), UiEdition::Bedrock);
            page.output = Some(out);
            BEDROCK_INDEX.store(index, std::sync::atomic::Ordering::Relaxed);
            assert!(page.has_replace_step(), "{out:?}");
            page.step = 3;
            let start = Instant::now();
            while page.scanned.is_none() && page.scan_error.is_none() {
                frame(&ctx, &mut page, &mut settings);
                assert!(start.elapsed().as_secs() < 180, "scan did not finish");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(page.scan_error.is_none(), "{out:?}: {:?}", page.scan_error);
            let plan = &page.scanned.as_ref().unwrap().plan;
            eprintln!("{out:?}: {} kinds of blocks to replace", plan.groups.len());
            let dst = std::env::temp_dir().join(format!("mctk-gui-bedrock-{out:?}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dst);
            let text = route_job(
                UiEdition::Bedrock,
                out,
                std::path::Path::new(&world),
                &dst,
                "26.3",
                &page.scanned.as_ref().unwrap().plan.to_map(),
                &ConvertOptions::default(),
                &Progress::default(),
            )
            .unwrap_or_else(|e| panic!("{out:?}: {e}"));
            eprintln!("{out:?}: {}", text.lines().next().unwrap_or(""));
            let _ = std::fs::remove_dir_all(&dst);
        }
        BEDROCK_INDEX.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn replace_step_scans_renders_and_feeds_the_converter() {
        let Ok(world) = std::env::var("MCTK_TEST_JAVA_WORLD") else {
            return;
        };
        let ctx = egui::Context::default();
        theme::install(&ctx);
        icons::install_textures(&ctx);
        let mut settings = Settings::default();
        let mut page = ConverterPage::new(&settings);
        page.set_input(PathBuf::from(&world), UiEdition::Java);
        page.output = Some(UiEdition::Java);
        page.java_version = ConverterPage::java_versions().iter().position(|v| *v == "1.20.1").unwrap();
        page.step = 3;
        assert!(page.java_to_java() && page.convert_idx() == 4);

        // The scan runs on a worker thread; keep drawing frames until it lands.
        let start = Instant::now();
        while page.scanned.is_none() && page.scan_error.is_none() {
            assert!(frame(&ctx, &mut page, &mut settings) > 0);
            assert!(start.elapsed().as_secs() < 60, "scan did not finish");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(page.scan_error.is_none(), "{:?}", page.scan_error);
        let plan = &page.scanned.as_ref().unwrap().plan;
        assert!(plan.groups.iter().any(|g| g.name == "minecraft:short_grass"), "{:?}", plan.groups.iter().map(|g| &g.name).collect::<Vec<_>>());
        assert!(plan.groups.iter().any(|g| g.name == "minecraft:leaf_litter"));
        frame(&ctx, &mut page, &mut settings);

        // The user turns leaf litter into grass instead of removing it.
        let i = page.scanned.as_ref().unwrap().plan.groups.iter().position(|g| g.name == "minecraft:leaf_litter").unwrap();
        page.scanned.as_mut().unwrap().plan.groups[i].choice = Choice::Block("minecraft:grass".into());
        frame(&ctx, &mut page, &mut settings);

        let map = page.scanned.as_ref().unwrap().plan.to_map();
        let dst = std::env::temp_dir().join(format!("mctk-gui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        let text = java_to_java_job(
            std::path::Path::new(&world),
            &dst,
            "1.20.1",
            &map,
            &ConvertOptions::default(),
            &Progress::default(),
        )
        .unwrap();
        assert!(text.lines().next().unwrap().parse::<usize>().unwrap() > 0, "{text}");

        // Scan the converted world: leaf litter must now be grass, and nothing unsupported left.
        let target = mctk_registry::builtin::java("1.20.1").unwrap();
        let scan = mctk_replace::Scan::java_world(&dst).unwrap();
        let _ = std::fs::remove_dir_all(&dst);
        assert!(scan.unsupported(&target).is_empty());
        assert!(!scan.blocks.keys().any(|b| b.name == "minecraft:leaf_litter"));
    }

    /// The Bedrock route: the scan uses the Java-side view of Bedrock, an override reaches the
    /// converter, and the Bedrock world that comes out holds the block that was chosen.
    #[test]
    fn bedrock_replace_step_scans_and_the_choice_reaches_the_world() {
        let Ok(world) = std::env::var("MCTK_TEST_JAVA_WORLD") else {
            return;
        };
        let ctx = egui::Context::default();
        theme::install(&ctx);
        icons::install_textures(&ctx);
        let mut settings = Settings::default();
        let mut page = ConverterPage::new(&settings);
        page.set_input(PathBuf::from(&world), UiEdition::Java);
        page.output = Some(UiEdition::Bedrock);
        page.step = 3;
        assert!(page.has_replace_step() && !page.java_to_java() && page.convert_idx() == 4);

        let start = Instant::now();
        while page.scanned.is_none() && page.scan_error.is_none() {
            frame(&ctx, &mut page, &mut settings);
            assert!(start.elapsed().as_secs() < 90, "scan did not finish");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(page.scan_error.is_none(), "{:?}", page.scan_error);
        let plan = &mut page.scanned.as_mut().unwrap().plan;
        let i = plan
            .groups
            .iter()
            .position(|g| g.name == "minecraft:black_wool_slab")
            .expect("a double wool slab needs a decision on Bedrock");
        // The smart default is the full block; the user picks concrete instead.
        assert_eq!(plan.groups[i].suggestion_text(), "minecraft:black_wool");
        plan.groups[i].choice = Choice::Block("minecraft:black_concrete".into());
        frame(&ctx, &mut page, &mut settings);

        let map = page.scanned.as_ref().unwrap().plan.to_map();
        let dst = std::env::temp_dir().join(format!("mctk-gui-bedrock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        let text = java_to_bedrock_job(std::path::Path::new(&world), &dst, &map, &ConvertOptions::default(), &Progress::default()).unwrap();
        assert!(text.lines().next().unwrap().parse::<usize>().unwrap() > 0, "{text}");
        assert!(text.contains("replaced: minecraft:black_wool_slab"), "{text}");

        let converted = mctk_bedrock::BedrockWorld::open(&dst).unwrap();
        let mut names = std::collections::BTreeSet::new();
        for (key, bytes) in &converted.db.entries {
            if let mctk_bedrock::Key::Chunk(ck) = mctk_bedrock::Key::decode(key) {
                if ck.tag == mctk_bedrock::ChunkTag::SubChunkPrefix {
                    if let Some(s) = mctk_bedrock::SubChunk::parse(bytes).ok().as_ref().and_then(mctk_bedrock::SubChunk::blocks) {
                        names.extend(s.palette.iter().filter_map(|e| e.tag.get("name").and_then(|n| n.as_str()).map(str::to_string)));
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dst);
        assert!(names.contains("minecraft:black_concrete"), "the chosen block is in the world");
        assert!(!names.contains("minecraft:black_wool_double_slab"));
    }
    /// The Bedrock input route: the scan finds the Bedrock-only blocks, an override reaches the
    /// converter, and the Java world that comes out holds the chosen block.
    #[test]
    fn bedrock_input_replace_step_scans_and_the_choice_reaches_the_world() {
        let Ok(world) = std::env::var("MCTK_TEST_BEDROCK_WORLD") else {
            return;
        };
        let ctx = egui::Context::default();
        theme::install(&ctx);
        icons::install_textures(&ctx);
        let mut settings = Settings::default();
        let mut page = ConverterPage::new(&settings);
        page.set_input(PathBuf::from(&world), UiEdition::Bedrock);
        page.output = Some(UiEdition::Java);
        page.step = 3;
        assert!(page.has_replace_step() && !page.java_to_java() && page.convert_idx() == 4);
        assert_eq!(page.java_version_name(), mctk_registry::builtin::java_versions()[0], "Bedrock goes to the newest Java only");

        let start = Instant::now();
        while page.scanned.is_none() && page.scan_error.is_none() {
            frame(&ctx, &mut page, &mut settings);
            assert!(start.elapsed().as_secs() < 90, "scan did not finish");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(page.scan_error.is_none(), "{:?}", page.scan_error);
        let plan = &mut page.scanned.as_mut().unwrap().plan;
        let i = plan.groups.iter().position(|g| g.name == "minecraft:invisible_bedrock").expect("a Bedrock-only block needs a decision");
        assert_eq!(plan.groups[i].suggestion_text(), "remove");
        let default_map = plan.to_map();
        plan.groups[i].choice = Choice::Block("minecraft:bedrock".into());
        frame(&ctx, &mut page, &mut settings);
        let chosen_map = page.scanned.as_ref().unwrap().plan.to_map();

        let count_bedrock = |map: &ReplacementMap, tag: &str| -> u64 {
            let dst = std::env::temp_dir().join(format!("mctk-gui-b2j-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dst);
            let text = bedrock_to_java_job(std::path::Path::new(&world), &dst, map, &ConvertOptions::default(), &Progress::default()).unwrap();
            assert!(text.lines().next().unwrap().parse::<usize>().unwrap() > 0, "{text}");
            let scan = mctk_replace::Scan::java_world(&dst).unwrap();
            let _ = std::fs::remove_dir_all(&dst);
            scan.blocks.iter().filter(|(b, _)| b.name == "minecraft:bedrock").map(|(_, n)| *n).sum()
        };
        let (before, after) = (count_bedrock(&default_map, "default"), count_bedrock(&chosen_map, "chosen"));
        assert_eq!(after, before + 1, "the chosen block replaced the one block, nothing else");
    }}
