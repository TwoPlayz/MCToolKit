//! The splash screen: shown over the window while the app starts. It checks for a newer release
//! (and, when allowed, downloads and installs it), and checks the game's assets are current; the
//! app opens once both are done (and the screen has been up long enough to read).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, Vec2, pos2, vec2};

use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::{theme, update};

/// The least time it stays up, so the steps can be read.
const SHOWN_AT_LEAST: Duration = Duration::from_millis(1500);
/// The most it waits for the network; the app opens anyway and tries again next start.
const GIVE_UP: Duration = Duration::from_secs(12);

pub enum Outcome {
    Waiting,
    /// Done: the update found (that was not installed) and the notes for the log.
    Ready(Option<update::Update>, Vec<String>),
    /// The new version is in place: start it and close this one.
    Relaunch,
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Skipped,
    Running,
    Done,
}

pub struct Splash {
    started: Instant,
    updates: Option<Job<Option<update::Update>>>,
    updates_step: Step,
    resources: Option<Job<()>>,
    resources_step: Step,
    install: Option<Job<()>>,
    installing: Option<update::Update>,
    progress: Arc<update::Progress>,
    found: Option<update::Update>,
    notes: Vec<String>,
    /// What the update check said, for its line on the screen.
    updates_line: Option<String>,
}

impl Splash {
    pub fn new(ctx: &egui::Context, settings: &crate::settings::Settings, pack: std::path::PathBuf) -> Self {
        let updates = settings.check_updates.then(|| Job::spawn("Update check", ctx, update::check));
        let resources = Job::spawn("Resources", ctx, move || {
            crate::resource_pack::prepare(&pack);
            Ok(())
        });
        Splash {
            started: Instant::now(),
            updates_step: if updates.is_some() { Step::Running } else { Step::Skipped },
            updates,
            resources: Some(resources),
            resources_step: Step::Running,
            install: None,
            installing: None,
            progress: Arc::default(),
            found: None,
            notes: Vec::new(),
            updates_line: None,
        }
    }

    pub fn step(&mut self, ctx: &egui::Context) -> Outcome {
        ctx.request_repaint_after(Duration::from_millis(33));
        if let Some(r) = self.updates.as_ref().and_then(Job::take) {
            self.updates = None;
            self.updates_step = Step::Done;
            match r {
                Ok(Some(u)) => {
                    self.notes.push(format!("MCToolKit {} is available", u.version));
                    if update::can_install(&u) {
                        let (job_update, progress) = (u.clone(), self.progress.clone());
                        self.install = Some(Job::spawn("Update", ctx, move || update::install(&job_update, &progress)));
                        self.updates_line = Some(format!("Update v{} found", u.version));
                        self.installing = Some(u);
                    } else {
                        self.updates_line = Some(format!("Update v{} is available", u.version));
                        self.found = Some(u);
                    }
                }
                Ok(None) => {
                    self.notes.push("MCToolKit is up to date".into());
                    self.updates_line = Some("MCToolKit is up to date".into());
                }
                Err(e) => {
                    self.notes.push(format!("Update check failed: {e}"));
                    self.updates_line = Some("Could not check for updates".into());
                }
            }
        }
        if self.resources.as_ref().and_then(Job::take).is_some() {
            self.resources = None;
            self.resources_step = Step::Done;
        }
        if let Some(r) = self.install.as_ref().and_then(Job::take) {
            self.install = None;
            match r {
                Ok(()) => return Outcome::Relaunch,
                Err(e) => {
                    self.notes.push(format!("The update was not installed: {e}"));
                    self.updates_line = Some("The update could not be installed".into());
                    self.found = self.installing.take();
                }
            }
        }
        let t = self.started.elapsed();
        let idle = self.install.is_none() && self.updates_step != Step::Running && self.resources_step != Step::Running;
        if (idle && t >= SHOWN_AT_LEAST) || (self.install.is_none() && t >= GIVE_UP) {
            return Outcome::Ready(self.found.take(), std::mem::take(&mut self.notes));
        }
        Outcome::Waiting
    }

    /// Draws the screen in `body` (below the title bar).
    pub fn draw(&self, ui: &mut egui::Ui, body: Rect) {
        let p = ui.painter();
        p.rect_filled(body, 0.0, theme::bg());
        let c = pos2(body.center().x, body.center().y - 36.0);
        icons::paint(p, Rect::from_center_size(c - vec2(0.0, 50.0), Vec2::splat(96.0)), Icon::Logo, Color32::WHITE);
        p.text(c + vec2(0.0, 26.0), Align2::CENTER_CENTER, "MCToolKit", theme::regular(32.0), theme::strong());
        p.text(c + vec2(0.0, 56.0), Align2::CENTER_CENTER, format!("Version {}", env!("CARGO_PKG_VERSION")), theme::regular(14.0), theme::text_muted());
        // The bar: how far the download is, or a stretch of it passing along.
        let bar = Rect::from_center_size(c + vec2(0.0, 96.0), vec2(320.0, 6.0));
        p.rect_filled(bar, 3.0, theme::button());
        let (done, total) = (self.progress.done.load(Ordering::Relaxed), self.progress.total.load(Ordering::Relaxed));
        if self.install.is_some() && total > 0 {
            let w = bar.width() * (done as f32 / total as f32).clamp(0.0, 1.0);
            p.rect_filled(Rect::from_min_size(bar.min, vec2(w, bar.height())), 3.0, theme::accent());
        } else {
            let t = ui.input(|i| i.time) as f32;
            let seg = 100.0;
            let x = (t * 0.9).rem_euclid(1.0) * (bar.width() + seg) - seg;
            let r = Rect::from_min_max(pos2(bar.left() + x.max(0.0), bar.top()), pos2(bar.left() + (x + seg).min(bar.width()), bar.bottom()));
            if r.width() > 0.0 {
                p.rect_filled(r, 3.0, theme::accent());
            }
        }
        // What it is doing, a line each.
        let megabytes = |n: u64| format!("{:.1}", n as f64 / 1_048_576.0);
        let mut lines: Vec<(Step, String)> = Vec::new();
        match (&self.installing, self.updates_step) {
            (Some(u), _) => lines.push((Step::Running, format!("Downloading v{}: {} of {} MB", u.version, megabytes(done), megabytes(total)))),
            (None, Step::Running) => lines.push((Step::Running, "Checking for updates".into())),
            (None, Step::Done) => lines.push((Step::Done, self.updates_line.clone().unwrap_or_default())),
            (None, Step::Skipped) => {}
        }
        lines.push(match self.resources_step {
            Step::Done => (Step::Done, "Game resources are ready".into()),
            _ => (Step::Running, "Checking game resources".into()),
        });
        let top = bar.bottom() + 26.0;
        for (i, (step, text)) in lines.iter().enumerate() {
            let y = top + i as f32 * 24.0;
            let galley = ui.painter().layout_no_wrap(text.clone(), theme::regular(14.0), theme::text());
            let width = galley.size().x + 26.0;
            let left = body.center().x - width / 2.0;
            let mark = Rect::from_center_size(pos2(left + 7.0, y), Vec2::splat(14.0));
            if *step == Step::Done {
                icons::paint(ui.painter(), mark, Icon::Check, theme::accent());
            } else {
                ui.put(mark, egui::Spinner::new().size(14.0).color(theme::accent()));
            }
            ui.painter().galley(pos2(left + 26.0, y - galley.size().y / 2.0), galley, theme::text());
        }
    }
}