//! MCToolKit desktop app: a frameless dark window, an app picker, and a sidebar with the NBT
//! Editor (its World Explorer prunes too), the Converter and Settings. Long tasks run on background threads.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod array_view;
mod atlas;
mod heads;
mod hex_editor;
mod info;
mod chunk_preview;
mod converter;
mod ctx_menu;
mod edit;
mod effects;
mod explorer_ui;
mod enchant;
mod assets;
mod entities;
mod mobs;
mod gpu_world;
mod models;
mod naming;
mod mob_edit;
mod item_edit;
mod trades;
mod inventory;
mod item_picker;
mod editor;
mod icons;
mod jobs;
mod map_view;
mod nbt_view;
mod pruner;
mod resource_pack;
mod settings;
#[cfg(test)]
mod snapshot;
mod street_view;
mod theme;
mod toasts;
mod update;
mod widgets;
mod workspace;
mod world;
mod world_picker;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Rect, ResizeDirection, Sense, Stroke, Vec2, ViewportCommand,
    pos2, vec2,
};

use icons::Icon;
use settings::Settings;
use world_picker::{Picked, UiEdition, WorldPicker};

const TITLE_BAR: f32 = 38.0;
const SIDEBAR: f32 = 70.0;
/// Sidebar, the editor/pruner left panel and a map wide enough for the Locate bar, the
/// dimension buttons and Slicer on one row.
/// The smallest window.
const MIN_WIDTH: f32 = 800.0;
const MIN_HEIGHT: f32 = 490.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Editor,
    Converter,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Editor,
    Converter,
}

struct App {
    page: Option<Page>,
    settings: Settings,
    editor: editor::EditorPage,
    converter: converter::ConverterPage,
    picker: Option<(WorldPicker, Purpose)>,
    log: Vec<String>,
    /// The messages in the corner for the pages other than the NBT editor (which has its own).
    toasts: toasts::Toasts,
    last_statuses: [String; 4],
    zoom: f32,
    update_check: Option<jobs::Job<Option<update::Update>>>,
    update: Option<update::Update>,
    /// Asking whether to save before closing; and the answer was to close (no asking again).
    confirm_quit: bool,
    quitting: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        theme::install(&cc.egui_ctx);
        icons::install_textures(&cc.egui_ctx);
        gpu_world::install(cc);
        let settings = Settings::load();
        // MCToolKit's own resource pack keeps the latest release's assets, fetched as they are
        // wanted (and again only when a newer release comes out).
        let pack = resource_pack::default_dir(&settings.working_dir());
        assets::set_root(pack.clone());
        std::thread::spawn(move || resource_pack::ensure(&pack));
        let page = match settings.default_app {
            1 => Some(Page::Editor),
            2 => Some(Page::Converter),
            _ => None,
        };
        let update_check = settings
            .check_updates
            .then(|| jobs::Job::spawn("Update check", &cc.egui_ctx, update::check));
        let mut app = App {
            page,
            update_check,
            update: None,
            converter: converter::ConverterPage::new(&settings),
            settings,
            editor: Default::default(),
            picker: None,
            log: vec![format!("MCToolKit {} started", env!("CARGO_PKG_VERSION"))],
            last_statuses: Default::default(),
            toasts: Default::default(),
            zoom: 0.0,
            confirm_quit: false,
            quitting: false,
        };
        // It closed by crashing last time: say so, and where the details are.
        let (log, pending) = crash_files();
        if pending.exists() {
            let _ = std::fs::remove_file(&pending);
            let text = format!("MCToolKit closed unexpectedly last time. What happened is in {}", log.display());
            app.log.push(text.clone());
            app.toasts.push(&cc.egui_ctx, text);
        }
        // For development and automated checks: MCTK_DEV_OPEN opens a Java world straight in the
        // World Explorer, and MCTK_DEV_MAP places its map (see `MapView::dev_preset`).
        if let Ok(path) = std::env::var("MCTK_DEV_OPEN") {
            app.page = Some(Page::Editor);
            app.editor.open_world(std::path::Path::new(&path), world_picker::UiEdition::Java, &app.settings);
            app.editor.dev_show_explorer(std::env::var("MCTK_DEV_MAP").ok().as_deref());
            // MCTK_DEV_QUIT: the "Save changes before closing?" question up (as on closing).
            app.confirm_quit = std::env::var("MCTK_DEV_QUIT").is_ok() && !app.unsaved_worlds().is_empty();
        }
        // MCTK_DEV_THEME=dark|light: that theme (for this run; not saved unless settings change).
        match std::env::var("MCTK_DEV_THEME").as_deref() {
            Ok("dark") => app.settings.theme = 1,
            Ok("light") => app.settings.theme = 2,
            _ => {}
        }
        // MCTK_DEV_CONVERT=<java world>;<older java version>: the converter at its REPLACE step.
        if let Ok(spec) = std::env::var("MCTK_DEV_CONVERT") {
            let parts: Vec<&str> = spec.split(';').collect();
            if let [path, version, rest @ ..] = parts.as_slice() {
                let step = rest.first().and_then(|s| s.parse().ok()).unwrap_or(3);
                app.converter.dev_replace(std::path::PathBuf::from(path), version, step);
                app.page = Some(Page::Converter);
            }
        }
        // MCTK_DEV_PAGE=home|settings|converter|editor: the page shown at start.
        if let Ok(p) = std::env::var("MCTK_DEV_PAGE") {
            app.page = match p.as_str() {
                "settings" => Some(Page::Settings),
                "converter" => Some(Page::Converter),
                "editor" => Some(Page::Editor),
                _ => None,
            };
        }
        app
    }

    /// The name of `path` and the tool that has it open, when a tool other than `purpose`'s does.
    fn open_elsewhere(&self, path: &std::path::Path, purpose: Purpose) -> Option<(String, &'static str)> {
        let converter = self.converter.input.iter().map(|(p, edition)| {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            (edition.subject(&name), p.clone())
        });
        let tools: [(Purpose, &'static str, Vec<(String, std::path::PathBuf)>); 2] = [
            (Purpose::Editor, "NBT editor", self.editor.open_worlds()),
            (Purpose::Converter, "converter", converter.collect()),
        ];
        tools
            .into_iter()
            .filter(|(p, _, _)| *p != purpose)
            .find_map(|(_, tool, worlds)| {
                worlds
                    .into_iter()
                    .find(|(_, open)| editor::same_path(open, path))
                    .map(|(name, _)| (name, tool))
            })
    }

    fn open_picker(&mut self, ctx: &egui::Context, edition: UiEdition, purpose: Purpose) {
        let mut p = WorldPicker::new(edition);
        p.refresh(ctx, &self.settings);
        self.picker = Some((p, purpose));
    }

    fn title_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let rect = Rect::from_min_size(ui.max_rect().min, vec2(ui.max_rect().width(), TITLE_BAR));
        let drag = ui.interact(rect, egui::Id::new("title-bar"), Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(rect, 0.0, theme::bg());
        // Inside the bar's last pixel row, so the page drawn after it does not cover half of it.
        p.line_segment(
            [rect.left_bottom() - vec2(0.0, 0.5), rect.right_bottom() - vec2(0.0, 0.5)],
            Stroke::new(1.0_f32, theme::separator()),
        );
        icons::paint(
            p,
            Rect::from_center_size(rect.left_center() + vec2(22.0, 0.0), Vec2::splat(20.0)),
            Icon::Logo,
            Color32::WHITE,
        );
        p.text(
            rect.left_center() + vec2(42.0, 0.0),
            Align2::LEFT_CENTER,
            "MCToolKit",
            theme::regular(17.0),
            theme::strong(),
        );
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let mut x = rect.right();
        for (icon, id) in [
            (Icon::Close, "close"),
            (
                if maximized {
                    Icon::Restore
                } else {
                    Icon::Maximize
                },
                "max",
            ),
            (Icon::Minimize, "min"),
        ] {
            let r = Rect::from_min_size(pos2(x - 46.0, rect.top()), vec2(46.0, TITLE_BAR - 1.0));
            x -= 46.0;
            let resp = ui.interact(r, egui::Id::new(("win", id)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(
                    r,
                    0.0,
                    if id == "close" {
                        Color32::from_rgb(0xC4, 0x2B, 0x1C)
                    } else {
                        theme::button_hover()
                    },
                );
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(r.center(), Vec2::splat(18.0)),
                icon,
                theme::strong(),
            );
            if resp.clicked() {
                match id {
                    "close" => ctx.send_viewport_cmd(ViewportCommand::Close),
                    "max" => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                    _ => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                }
            }
        }
        if let Some(u) = &self.update {
            let font = theme::regular(13.0);
            let text = format!("Update available: v{}", u.version);
            let galley = ui
                .painter()
                .layout_no_wrap(text, font, Color32::WHITE);
            let size = vec2(galley.size().x + 24.0, 24.0);
            let r = Rect::from_center_size(pos2(x - 12.0 - size.x / 2.0, rect.center().y), size);
            let resp = ui
                .interact(r, egui::Id::new("update-pill"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Open the download page on GitHub");
            ui.painter().rect_filled(
                r,
                theme::radius(4),
                if resp.hovered() { theme::button_hover() } else { theme::button() },
            );
            ui.painter().galley(
                pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0),
                galley,
                theme::strong(),
            );
            if resp.clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(&u.url));
            }
        }
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        } else if drag.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    /// Resize handles along the edges of the frameless window.
    fn resize_edges(ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let r = ctx.screen_rect();
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let m = 5.0;
        let (l, rt, t, b) = (
            pos.x - r.left() < m,
            r.right() - pos.x < m,
            pos.y - r.top() < m,
            r.bottom() - pos.y < m,
        );
        let dir = match (l, rt, t, b) {
            (true, _, true, _) => Some((ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest)),
            (_, true, true, _) => Some((ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast)),
            (true, _, _, true) => Some((ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest)),
            (_, true, _, true) => Some((ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast)),
            (true, ..) => Some((ResizeDirection::West, CursorIcon::ResizeWest)),
            (_, true, ..) => Some((ResizeDirection::East, CursorIcon::ResizeEast)),
            (_, _, true, _) => Some((ResizeDirection::North, CursorIcon::ResizeNorth)),
            (.., true) => Some((ResizeDirection::South, CursorIcon::ResizeSouth)),
            _ => None,
        };
        if let Some((d, cursor)) = dir {
            ctx.set_cursor_icon(cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        let rect = Rect::from_min_size(full.min, vec2(SIDEBAR, full.height()));
        ui.painter().rect_filled(rect, 0.0, theme::bg());
        ui.painter().line_segment(
            [rect.right_top() - vec2(0.5, 0.0), rect.right_bottom() - vec2(0.5, 0.0)],
            Stroke::new(1.0_f32, theme::separator()),
        );
        let item = |ui: &mut egui::Ui,
                    top: f32,
                    icon: Icon,
                    label: &str,
                    page: Page,
                    accent: Color32,
                    current: Option<Page>|
         -> bool {
            let r = Rect::from_min_size(pos2(rect.left(), top), vec2(SIDEBAR, 64.0));
            let resp = ui.interact(r, egui::Id::new(("side", label)), Sense::click());
            let selected = current == Some(page);
            let p = ui.painter();
            if selected {
                p.rect_filled(r, 0.0, theme::deep());
                p.rect_filled(
                    Rect::from_min_size(r.min, vec2(2.5, r.height())),
                    0.0,
                    accent,
                );
            } else if resp.hovered() {
                p.rect_filled(r, 0.0, theme::card());
            }
            let ic = if selected { accent } else { theme::text_muted() };
            icons::paint(
                p,
                Rect::from_center_size(r.center() - vec2(0.0, 9.0), Vec2::splat(30.0)),
                icon,
                ic,
            );
            let tc = if selected {
                theme::strong()
            } else {
                theme::text_muted()
            };
            let font = if selected {
                theme::bold(10.5)
            } else {
                theme::regular(10.5)
            };
            p.text(
                r.center() + vec2(0.0, 18.0),
                Align2::CENTER_CENTER,
                label,
                font,
                tc,
            );
            resp.clicked()
        };
        let current = self.page;
        let top = rect.top();
        for (i, (icon, label, page, accent, _)) in [
            (
                Icon::AppEditor,
                "NBT Editor",
                Page::Editor,
                theme::EDITOR_ACCENT,
                "View and edit the raw saved data of a Minecraft world in NBT format.",
            ),
            (
                Icon::AppConverter,
                "Converter",
                Page::Converter,
                theme::CONVERTER_ACCENT,
                "Convert a Minecraft world to a different edition and/or version.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if item(
                ui,
                top + i as f32 * 64.0,
                icon,
                label,
                page,
                accent,
                current,
            ) {
                self.page = Some(page);
            }
        }
        let bottom = rect.bottom() - 30.0;
        if item(
            ui,
            bottom - 64.0,
            Icon::Gear,
            "Settings",
            Page::Settings,
            theme::strong(),
            current,
        ) {
            self.page = Some(Page::Settings);
        }
        ui.painter().text(
            pos2(rect.center().x, rect.bottom() - 14.0),
            Align2::CENTER_CENTER,
            env!("CARGO_PKG_VERSION"),
            theme::regular(11.5),
            theme::text_muted(),
        );
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let c = ui.max_rect().center();
        ui.painter().text(
            c - vec2(0.0, 128.0),
            Align2::CENTER_CENTER,
            "Select an App",
            theme::regular(20.0),
            theme::strong(),
        );
        // The hovered app's description under the cards.
        let mut description = "";
        for (i, (icon, label, page, accent, about)) in [
            (
                Icon::AppEditor,
                "NBT Editor",
                Page::Editor,
                theme::EDITOR_ACCENT,
                "View and edit the raw saved data of a Minecraft world in NBT format.",
            ),
            (
                Icon::AppConverter,
                "Converter",
                Page::Converter,
                theme::CONVERTER_ACCENT,
                "Convert a Minecraft world to a different edition and/or version.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            // (Two cards, side by side about the middle.)
            let r = Rect::from_center_size(
                c + vec2((i as f32 - 0.5) * 212.0, 10.0),
                vec2(192.0, 192.0),
            );
            let resp = ui
                .interact(r, egui::Id::new(("home", label)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let p = ui.painter();
            p.rect_filled(
                r,
                theme::radius(6),
                if resp.hovered() {
                    theme::button_hover()
                } else {
                    theme::card()
                },
            );
            p.rect_stroke(
                r,
                theme::radius(6),
                Stroke::new(
                    1.0_f32,
                    if resp.hovered() {
                        theme::chip()
                    } else {
                        theme::border()
                    },
                ),
                egui::StrokeKind::Inside,
            );
            icons::paint(
                p,
                Rect::from_center_size(r.center() - vec2(0.0, 18.0), Vec2::splat(80.0)),
                icon,
                accent,
            );
            p.text(
                r.center() + vec2(0.0, 54.0),
                Align2::CENTER_CENTER,
                label,
                theme::regular(15.0),
                theme::text(),
            );
            if resp.hovered() {
                description = about;
            }
            if resp.clicked() {
                self.page = Some(page);
            }
        }
        ui.painter().text(
            c + vec2(0.0, 140.0),
            Align2::CENTER_CENTER,
            description,
            theme::regular(15.0),
            theme::text_muted(),
        );
    }

    /// The names of the open worlds with changes not saved.
    fn unsaved_worlds(&self) -> Vec<String> {
        self.editor.workspaces.iter().filter(|w| w.dirty()).map(|w| w.name.clone()).collect()
    }

    /// "Save changes before closing?": Save writes every world with changes and closes (unless
    /// one fails, which is said and the app stays open); Don't Save closes; Cancel stays.
    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_quit {
            return;
        }
        let names = self.unsaved_worlds();
        let which = match names.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            many => format!("{} worlds ({})", many.len(), many.join(", ")),
        };
        let answer = editor::dialog(
            ctx,
            "confirm-quit",
            "Save changes before closing?",
            &[(format!("{which} has changes that are not saved."), false), ("Closing without saving loses them.".into(), true)],
            &[(Some(Icon::Save), "Save"), (None, "Don't Save"), (None, "Cancel")],
            true,
        );
        match answer {
            Some(Some(0)) => {
                let mut failed = None;
                for ws in self.editor.workspaces.iter_mut().filter(|w| w.dirty()) {
                    if let Err(e) = ws.save() {
                        failed = Some(format!("Could not save {}: {e}", ws.name));
                        break;
                    }
                }
                self.confirm_quit = false;
                match failed {
                    Some(e) => {
                        self.log.push(e.clone());
                        self.toasts.push(ctx, e);
                    }
                    None => {
                        self.quitting = true;
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
            Some(Some(1)) => {
                self.confirm_quit = false;
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Some(_) => self.confirm_quit = false,
            None => {}
        }
    }

    /// Collects page status messages into the debug log.
    fn collect_log(&mut self, ctx: &egui::Context) {
        // The editor shows and takes its own messages; the other pages' go to the corner here.
        let s = &self.editor.status;
        if !s.is_empty() && *s != self.last_statuses[0] {
            self.log.push(s.clone());
            self.last_statuses[0] = s.clone();
        }
        let status = &mut self.converter.status;
        if !status.is_empty() {
            let text = std::mem::take(status);
            self.log.push(text.clone());
            self.toasts.push(ctx, text);
        }
        if self.log.len() > 500 {
            self.log.drain(..100);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.zoom == 0.0 {
            round_corners(frame);
        }
        // Anything that is only clicked (buttons, close crosses, list rows, ...) shows the
        // pointing hand. Widgets that want another cursor set theirs later and win.
        let hovered: Vec<egui::Id> = ctx.interaction_snapshot(|s| s.hovered.iter().copied().collect());
        let over_button = hovered.into_iter().any(|id| {
            ctx.read_response(id)
                .is_some_and(|r| r.enabled() && r.sense.senses_click() && !r.sense.senses_drag())
        });
        if over_button {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // Each page has its own accent color for highlights, buttons and switches.
        let accent = match self.page {
            Some(Page::Editor) => theme::Accent::Green,
            _ => theme::Accent::Blue,
        };
        theme::set_accent(accent);
        ctx.style_mut(|s| s.visuals.selection.bg_fill = theme::accent());
        toasts::set_only_warnings(self.settings.hide_messages);
        // The theme the settings ask for (System: as Windows is set, followed while running).
        let light = match self.settings.theme {
            1 => false,
            2 => true,
            _ => ctx.system_theme() == Some(egui::Theme::Light),
        };
        if light != theme::is_light() {
            theme::set_light(ctx, light);
        }
        if self.zoom != self.settings.zoom() {
            self.zoom = self.settings.zoom();
            ctx.set_zoom_factor(self.zoom);
        }
        // Closing with unsaved changes in a world: not yet, first ask (from the taskbar too, so
        // before the minimized window's early return; it is brought back up to ask).
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && !self.unsaved_worlds().is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            self.confirm_quit = true;
        }
        // Minimized: nothing to draw (and the window's place and size then are Windows' own,
        // far off screen and of no size, not to be remembered).
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized {
            return;
        }
        if self.settings.remember_position {
            if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
                let w = [r.left(), r.top(), r.width(), r.height()];
                let sane = r.left() > -10_000.0 && r.top() > -10_000.0 && r.width() >= 200.0 && r.height() >= 150.0;
                if sane
                    && self.settings.window != Some(w)
                    && !ctx.input(|i| i.viewport().maximized.unwrap_or(false))
                {
                    self.settings.window = Some(w);
                }
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                self.settings.save();
            }
        }
        if let Some(r) = self.update_check.as_ref().and_then(jobs::Job::take) {
            self.update_check = None;
            match r {
                Ok(Some(u)) => {
                    self.log.push(format!("MCToolKit {} is available", u.version));
                    self.update = Some(u);
                }
                Ok(None) => self.log.push("MCToolKit is up to date".into()),
                Err(e) => self.log.push(format!("Update check failed: {e}")),
            }
        }
        // A finished conversion, prune or save blinks the taskbar button when the window is
        // in the background; Windows stops it once the window is focused again.
        if jobs::take_finished() && !ctx.input(|i| i.focused) {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }
        Self::resize_edges(ctx);
        self.quit_dialog(ctx);
        self.collect_log(ctx);
        self.toasts.show(ctx);
        egui::CentralPanel::default()
            // Windows draws the rounded outline (see `round_corners`).
            .frame(egui::Frame::new().fill(theme::bg()))
            .show(ctx, |ui| {
                self.title_bar(ctx, ui);
                let full = ui.max_rect();
                let mut body =
                    Rect::from_min_max(pos2(full.left(), full.top() + TITLE_BAR), full.max);
                let Some(page) = self.page else {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
                    self.home(&mut child);
                    return;
                };
                let mut side = ui.new_child(egui::UiBuilder::new().max_rect(body));
                self.sidebar(&mut side);
                body.min.x += SIDEBAR;
                if self.settings.show_log {
                    let (log_rect, rest) = if self.settings.log_right {
                        let x = body.right() - 300.0;
                        (
                            Rect::from_min_max(pos2(x, body.top()), body.max),
                            Rect::from_min_max(body.min, pos2(x, body.bottom())),
                        )
                    } else {
                        let y = body.bottom() - 130.0;
                        (
                            Rect::from_min_max(pos2(body.left(), y), body.max),
                            Rect::from_min_max(body.min, pos2(body.right(), y)),
                        )
                    };
                    ui.painter().rect_filled(log_rect, 0.0, theme::deep());
                    let mut child =
                        ui.new_child(egui::UiBuilder::new().id_salt("debug-log").max_rect(log_rect.shrink(6.0)));
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink(false)
                        .show(&mut child, |ui| {
                            for line in &self.log {
                                ui.label(
                                    egui::RichText::new(line)
                                        .monospace()
                                        .size(12.0)
                                        .color(theme::text_muted()),
                                );
                            }
                        });
                    body = rest;
                }
                // (Each child needs its own id, or their scroll areas share one state.)
                let mut child = ui.new_child(egui::UiBuilder::new().id_salt("page").max_rect(body));
                child.set_clip_rect(body);
                match page {
                    Page::Editor => {
                        if let editor::EditorRequest::PickWorld(e) =
                            self.editor.ui(&mut child, &mut self.settings)
                        {
                            self.open_picker(ctx, e, Purpose::Editor);
                        }
                    }
                    Page::Converter => {
                        if let converter::ConverterRequest::PickWorld(e) =
                            self.converter.ui(&mut child, &mut self.settings)
                        {
                            self.open_picker(ctx, e, Purpose::Converter);
                        }
                    }
                    Page::Settings => {
                        settings::page(&mut child, &mut self.settings);
                    }
                }
            });
        if let Some((picker, purpose)) = &mut self.picker {
            match picker.show(ctx, &mut self.settings) {
                Picked::Nothing => {}
                Picked::Cancelled => self.picker = None,
                Picked::Open(path) => {
                    let (edition, purpose) = (picker.edition, *purpose);
                    self.picker = None;
                    // A world is open in one place at a time, or one tool would work on a file
                    // the other has since changed.
                    if let Some((name, tool)) = self.open_elsewhere(&path, purpose) {
                        // The converter gives way: a conversion is a series of steps that can be
                        // started again, and the NBT editor and the pruner are for working on a
                        // world. (Not while it is being converted.)
                        let converter_lets_go = tool == "converter" && purpose != Purpose::Converter && self.converter.reset();
                        let text = if converter_lets_go {
                            format!("{name} was closed in the converter.")
                        } else {
                            format!("{name} is already open in the {tool}, so it was not opened again.")
                        };
                        self.log.push(text.clone());
                        self.toasts.push(ctx, text);
                        if !converter_lets_go {
                            return;
                        }
                    }
                    self.log.push(format!("Opening {}", path.display()));
                    match purpose {
                        Purpose::Editor => self.editor.open_world(&path, edition, &self.settings),
                        Purpose::Converter => self.converter.set_input(path, edition),
                    }
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
    }
}

/// Asks Windows 11 to round the corners of the frameless window (and give it the system
/// outline and shadow). Older Windows ignores the request.
#[cfg(windows)]
fn round_corners(frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const u32, size: u32) -> i32;
    }
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    if let RawWindowHandle::Win32(w) = handle.as_raw() {
        // SAFETY: a documented DWM call on our own top-level window with a u32 argument.
        unsafe {
            DwmSetWindowAttribute(
                w.hwnd.get(),
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_ROUND,
                4,
            );
        }
    }
}

#[cfg(not(windows))]
fn round_corners(_frame: &eframe::Frame) {}

/// The app's icon (taskbar/Alt-Tab/window decoration), matching `Icon::Logo` and the exe
/// icon embedded by `build.rs` (see `examples/gen_logo.rs`).
fn window_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/logo.png");
    let img = image::load_from_memory(bytes)
        .expect("assets/logo.png")
        .to_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

/// Where a crash is written down (`crash.log` in the app's settings folder), and the mark left
/// for the next start to say so.
fn crash_files() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = settings::app_dir();
    (dir.join("crash.log"), dir.join("crash_pending"))
}

/// Adds a line to `crash.log` (something that went wrong without closing the app).
pub fn crash_note(text: &str) {
    use std::io::Write;
    let (log, _) = crash_files();
    let _ = std::fs::create_dir_all(log.parent().unwrap_or(std::path::Path::new(".")));
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "[{secs}] {text}");
    }
}

/// On a crash: what happened, where, and the call stack, written to `crash.log`, and a mark for
/// the next start to tell the user.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let trace = std::backtrace::Backtrace::force_capture();
        crash_note(&format!("MCToolKit {} crashed on thread '{thread}': {info}\n{trace}", env!("CARGO_PKG_VERSION")));
        // Only the main thread's crash closes the app (a worker's is caught where it is used).
        if thread == "main" {
            let _ = std::fs::write(crash_files().1, b"1");
        }
        default(info);
    }));
}

fn main() -> eframe::Result<()> {
    install_crash_log();
    let settings = Settings::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MCToolKit")
        .with_icon(window_icon())
        .with_decorations(false)
        .with_inner_size([1250.0, 686.0])
        .with_min_inner_size([MIN_WIDTH, MIN_HEIGHT]);
    // (A place saved while minimized, far off screen, is not used.)
    if let (true, Some([x, y, w, h])) = (settings.remember_position, settings.window.filter(|w| w[0] > -10_000.0 && w[1] > -10_000.0)) {
        viewport = viewport
            .with_position([x, y])
            .with_inner_size([w.max(MIN_WIDTH), h.max(MIN_HEIGHT)]);
    }
    // For development: MCTK_DEV_SIZE=w,h opens the window that size (for screenshots).
    if let Some((w, h)) = std::env::var("MCTK_DEV_SIZE").ok().and_then(|s| s.split_once(',').and_then(|(w, h)| Some((w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?)))) {
        viewport = viewport.with_position([0.0, 0.0]).with_inner_size([w, h]);
    }
    // Drawn with wgpu (Vulkan or Direct3D 12) so the world viewer's 3D runs on the GPU, with a
    // depth buffer for it; OpenGL if wgpu cannot start (MCTK_RENDERER=gl forces it).
    let gl_only = std::env::var("MCTK_RENDERER").is_ok_and(|r| r == "gl");
    if !gl_only {
        let options = eframe::NativeOptions {
            viewport: viewport.clone(),
            renderer: eframe::Renderer::Wgpu,
            depth_buffer: 24,
            wgpu_options: gpu_world::wgpu_options(),
            ..Default::default()
        };
        match eframe::run_native("mctoolkit", options, Box::new(|cc| Ok(Box::new(App::new(cc))))) {
            Ok(()) => return Ok(()),
            Err(e) => eprintln!("wgpu could not start ({e}); drawing with OpenGL"),
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "mctoolkit",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::world::{self, Edition};

    const WORLDS: &str = r"C:\Users\JustinPC\Desktop\Worlds";

    /// Renders every sample world's overworld map (skipped when the samples are absent).
    #[test]
    fn maps_render_for_sample_worlds() {
        for (rel, edition) in [
            (r"Java\New World", Edition::Java),
            (r"BedrockPC\My World.mcworld", Edition::Bedrock),
            (r"PS3\NPUB31419--131226152509", Edition::Console),
            (r"XBO360\savegame.dat", Edition::Console),
        ] {
            let path = Path::new(WORLDS).join(rel);
            if !path.exists() {
                continue;
            }
            assert_eq!(world::detect(&path), Some(edition), "{rel}");
            let img = world::render_map(&path, edition, "minecraft:overworld", None).unwrap();
            let painted = img.rgba.chunks(4).filter(|p| p[3] == 255).count();
            assert!(painted > 0, "{rel}: nothing painted");
            // The slicer only ever removes blocks from the top.
            let sliced = world::render_map(&path, edition, "minecraft:overworld", Some(0)).unwrap();
            assert_eq!(sliced.width, img.width, "{rel}");
        }
    }
}
