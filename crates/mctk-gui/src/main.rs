//! MCToolKit desktop app, laid out like Universal Minecraft Tool: a frameless dark window,
//! an app picker, and a sidebar with the NBT Editor, Converter and Pruner plus Extensions
//! and Settings. Long tasks run on background threads.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod builtin;
mod converter;
mod editor;
mod extensions;
mod icons;
mod jobs;
mod map_view;
mod nbt_view;
mod pruner;
mod resource_pack;
mod settings;
mod theme;
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
const MIN_WIDTH: f32 = 1040.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Editor,
    Converter,
    Pruner,
    Extensions,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Editor,
    Converter,
    Pruner,
}

struct App {
    page: Option<Page>,
    settings: Settings,
    editor: editor::EditorPage,
    converter: converter::ConverterPage,
    pruner: pruner::PrunerPage,
    extensions: extensions::ExtensionsPage,
    picker: Option<(WorldPicker, Purpose)>,
    log: Vec<String>,
    last_statuses: [String; 4],
    zoom: f32,
    update_check: Option<jobs::Job<Option<update::Update>>>,
    update: Option<update::Update>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        theme::install(&cc.egui_ctx);
        icons::install_textures(&cc.egui_ctx);
        let settings = Settings::load();
        // Fill MCToolKit's own resource pack on first run (never a pack the user chose).
        let pack = resource_pack::default_dir(&settings.working_dir());
        if std::path::Path::new(&settings.resource_pack) == pack {
            std::thread::spawn(move || resource_pack::ensure(&pack));
        }
        let page = match settings.default_app {
            1 => Some(Page::Editor),
            2 => Some(Page::Converter),
            3 => Some(Page::Pruner),
            _ => None,
        };
        let update_check = settings
            .check_updates
            .then(|| jobs::Job::spawn("Update check", &cc.egui_ctx, update::check));
        App {
            page,
            update_check,
            update: None,
            converter: converter::ConverterPage::new(&settings),
            settings,
            editor: Default::default(),
            pruner: Default::default(),
            extensions: Default::default(),
            picker: None,
            log: vec![format!("MCToolKit {} started", env!("CARGO_PKG_VERSION"))],
            last_statuses: Default::default(),
            zoom: 0.0,
        }
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
        p.rect_filled(rect, 0.0, theme::BG);
        p.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0_f32, theme::BORDER),
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
            Color32::WHITE,
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
                        theme::BUTTON_HOVER
                    },
                );
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(r.center(), Vec2::splat(18.0)),
                icon,
                Color32::WHITE,
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
                if resp.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON },
            );
            ui.painter().galley(
                pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0),
                galley,
                Color32::WHITE,
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
        ui.painter().rect_filled(rect, 0.0, theme::BG);
        ui.painter().line_segment(
            [rect.right_top(), rect.right_bottom()],
            Stroke::new(1.0_f32, theme::BORDER),
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
                p.rect_filled(r, 0.0, theme::DEEP);
                p.rect_filled(
                    Rect::from_min_size(r.min, vec2(2.5, r.height())),
                    0.0,
                    accent,
                );
            } else if resp.hovered() {
                p.rect_filled(r, 0.0, theme::CARD);
            }
            let ic = if selected { accent } else { theme::TEXT_MUTED };
            icons::paint(
                p,
                Rect::from_center_size(r.center() - vec2(0.0, 9.0), Vec2::splat(30.0)),
                icon,
                ic,
            );
            let tc = if selected {
                Color32::WHITE
            } else {
                Color32::from_rgb(0xB8, 0xBE, 0xC6)
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
            (
                Icon::AppPruner,
                "Pruner",
                Page::Pruner,
                theme::PRUNER_ACCENT,
                "Select and delete unwanted areas of a Minecraft world.",
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
            bottom - 128.0,
            Icon::Extensions,
            "Extensions",
            Page::Extensions,
            Color32::WHITE,
            current,
        ) {
            self.page = Some(Page::Extensions);
        }
        if item(
            ui,
            bottom - 64.0,
            Icon::Gear,
            "Settings",
            Page::Settings,
            Color32::WHITE,
            current,
        ) {
            self.page = Some(Page::Settings);
        }
        ui.painter().text(
            pos2(rect.center().x, rect.bottom() - 14.0),
            Align2::CENTER_CENTER,
            env!("CARGO_PKG_VERSION"),
            theme::regular(11.5),
            theme::TEXT_MUTED,
        );
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let c = ui.max_rect().center();
        ui.painter().text(
            c - vec2(0.0, 128.0),
            Align2::CENTER_CENTER,
            "Select an App",
            theme::regular(20.0),
            Color32::WHITE,
        );
        // UMT shows the hovered app's description under the cards.
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
            (
                Icon::AppPruner,
                "Pruner",
                Page::Pruner,
                theme::PRUNER_ACCENT,
                "Select and delete unwanted areas of a Minecraft world.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Rect::from_center_size(
                c + vec2((i as f32 - 1.0) * 212.0, 10.0),
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
                    Color32::from_rgb(0x2B, 0x30, 0x3B)
                } else {
                    theme::CARD
                },
            );
            p.rect_stroke(
                r,
                theme::radius(6),
                Stroke::new(
                    1.0_f32,
                    if resp.hovered() {
                        Color32::from_rgb(0x33, 0x39, 0x42)
                    } else {
                        theme::BORDER
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
                Color32::from_rgb(0xC0, 0xC6, 0xCE),
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
            Color32::from_rgb(0x9E, 0xAE, 0xC2),
        );
    }

    /// Collects page status messages into the debug log.
    fn collect_log(&mut self) {
        let now = [
            &self.editor.status,
            &self.converter.status,
            &self.pruner.status,
            &self.extensions.status,
        ];
        for (i, s) in now.iter().enumerate() {
            if !s.is_empty() && *s != &self.last_statuses[i] {
                self.log.push((*s).clone());
                self.last_statuses[i] = (*s).clone();
            }
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
        icons::set_texture_pack(self.settings.texture_pack());
        if self.zoom != self.settings.zoom() {
            self.zoom = self.settings.zoom();
            ctx.set_zoom_factor(self.zoom);
        }
        if self.settings.remember_position {
            if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
                let w = [r.left(), r.top(), r.width(), r.height()];
                if self.settings.window != Some(w)
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
        self.collect_log();
        egui::CentralPanel::default()
            // Windows draws the rounded outline (see `round_corners`).
            .frame(egui::Frame::new().fill(theme::BG))
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
                    ui.painter().rect_filled(log_rect, 0.0, theme::DEEP);
                    let mut child =
                        ui.new_child(egui::UiBuilder::new().max_rect(log_rect.shrink(6.0)));
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink(false)
                        .show(&mut child, |ui| {
                            for line in &self.log {
                                ui.label(
                                    egui::RichText::new(line)
                                        .monospace()
                                        .size(12.0)
                                        .color(theme::TEXT_MUTED),
                                );
                            }
                        });
                    body = rest;
                }
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
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
                    Page::Pruner => {
                        if let pruner::PrunerRequest::PickWorld(e) =
                            self.pruner.ui(&mut child, &mut self.settings)
                        {
                            self.open_picker(ctx, e, Purpose::Pruner);
                        }
                    }
                    Page::Extensions => self.extensions.ui(&mut child, &mut self.settings),
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
                    self.log.push(format!("Opening {}", path.display()));
                    match purpose {
                        Purpose::Editor => self.editor.open_world(&path, edition, &self.settings),
                        Purpose::Converter => self.converter.set_input(path, edition),
                        Purpose::Pruner => self.pruner.open_world(&path, edition, &self.settings),
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
/// outline and shadow), as UMT's window has. Older Windows ignores the request.
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

fn main() -> eframe::Result<()> {
    let settings = Settings::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MCToolKit")
        .with_icon(window_icon())
        .with_decorations(false)
        .with_inner_size([1250.0, 686.0])
        .with_min_inner_size([MIN_WIDTH, 560.0]);
    if let (true, Some([x, y, w, h])) = (settings.remember_position, settings.window) {
        viewport = viewport
            .with_position([x, y])
            .with_inner_size([w.max(MIN_WIDTH), h.max(560.0)]);
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
