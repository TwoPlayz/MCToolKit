//! The NBT Editor page.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::icons::{self, Icon};
use crate::nbt_view::NbtTree;
use crate::settings::Settings;
use crate::theme;
use crate::widgets;
use crate::workspace::{self, DocSource, Workspace};
use crate::world_picker::UiEdition;

pub const LEFT_PANEL: f32 = 242.0;

#[derive(Default)]
pub struct EditorPage {
    pub workspaces: Vec<Workspace>,
    pub current: usize,
    /// Showing the "Select a World to Edit" chooser while worlds are open ("+").
    pub choosing: bool,
    more_open: bool,
    confirm_close: bool,
    pub status: String,
}

/// What the page asks the app to do.
pub enum EditorRequest {
    None,
    PickWorld(UiEdition),
}

/// The centred "Select a World to …" card with the five edition tiles. Returns the
/// edition clicked; `more` draws the optional "More ⌄" menu (NBT Editor only).
pub fn edition_chooser(
    ui: &mut Ui,
    title: &str,
    more: Option<&mut dyn FnMut(&mut Ui)>,
) -> Option<UiEdition> {
    let mut picked = None;
    let card = vec2(680.0, 202.0);
    let rect = Rect::from_center_size(ui.max_rect().center(), card);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        widgets::card(ui, 19, |ui| {
            ui.set_width(card.x - 40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(title).size(20.0).color(Color32::WHITE));
            });
            if let Some(more) = more {
                let r = Rect::from_min_size(
                    pos2(rect.right() - 80.0, rect.top() + 18.0),
                    vec2(60.0, 22.0),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
                more(&mut child);
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for e in UiEdition::ALL {
                    if widgets::tile(ui, e.icon(), e.label(), vec2(120.0, 120.0), false, true)
                        .clicked()
                    {
                        picked = Some(e);
                    }
                }
            });
        });
    });
    picked
}

impl EditorPage {
    pub fn open_world(&mut self, path: &Path, edition: UiEdition, settings: &Settings) {
        match Workspace::open(
            path,
            edition,
            settings,
            settings.editor_copy_java,
            settings.editor_copy_bedrock,
        ) {
            Ok(ws) => {
                self.status.clear();
                self.workspaces.push(ws);
                self.current = self.workspaces.len() - 1;
                self.choosing = false;
            }
            Err(e) => self.status = format!("Could not open the world: {e}"),
        }
    }

    fn open_file(&mut self, path: PathBuf, new: bool) {
        let tree = if new {
            Ok(NbtTree::new(mctk_nbt::NamedTag {
                name: String::new(),
                tag: mctk_nbt::Tag::Compound(Default::default()),
            }))
        } else {
            std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| crate::nbt_view::load(&b))
                .map(|(_, root)| NbtTree::new(root))
        };
        match tree {
            Ok(mut tree) => {
                if new {
                    tree.dirty = true;
                }
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let mut ws = Workspace::for_file(&path, &name);
                if let Err(e) = ws.open_doc(DocSource::File(path.clone())) {
                    if !new {
                        self.status = e;
                        return;
                    }
                    ws.push_new_doc(path, tree);
                }
                self.workspaces.push(ws);
                self.current = self.workspaces.len() - 1;
                self.choosing = false;
            }
            Err(e) => self.status = format!("Could not open {}: {e}", path.display()),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut Settings) -> EditorRequest {
        let mut request = EditorRequest::None;
        if self.workspaces.is_empty() || self.choosing {
            let full = ui.max_rect();
            if !self.workspaces.is_empty() {
                // Keep the left panel while choosing another world.
                self.left_panel(ui, settings);
            }
            let area = if self.workspaces.is_empty() {
                full
            } else {
                Rect::from_min_max(pos2(full.left() + LEFT_PANEL, full.top()), full.max)
            };
            let mut more_open = self.more_open;
            let mut file_action: Option<bool> = None;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
            let mut more = |ui: &mut Ui| {
                // One click target for the word and its arrow. Both turn white while the
                // pointer is over it or the menu is open, and the arrow points up while open.
                let (area, r) = ui.allocate_exact_size(vec2(60.0, 22.0), Sense::click());
                let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
                let color = if more_open || r.hovered() {
                    Color32::WHITE
                } else {
                    theme::TEXT_MUTED
                };
                ui.painter().text(
                    area.left_center(),
                    Align2::LEFT_CENTER,
                    "More",
                    theme::regular(13.5),
                    color,
                );
                let text_width = ui
                    .painter()
                    .layout_no_wrap("More".into(), theme::regular(13.5), color)
                    .size()
                    .x;
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(area.left() + text_width + 11.0, area.center().y + 1.0),
                        Vec2::splat(9.0),
                    ),
                    if more_open { Icon::ChevronUp } else { Icon::ChevronDown },
                    color,
                );                if r.clicked() {
                    more_open = !more_open;
                }
                if more_open {
                    egui::Area::new(egui::Id::new("more-menu"))
                        .fixed_pos(r.rect.left_bottom() + vec2(-70.0, 6.0))
                        .order(egui::Order::Foreground)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::new()
                                .fill(theme::DEEP)
                                .corner_radius(theme::radius(4))
                                .inner_margin(egui::Margin::same(6))
                                .show(ui, |ui| {
                                    if widgets::button(
                                        ui,
                                        Some(Icon::File),
                                        "Open NBT File...",
                                        150.0,
                                        true,
                                    )
                                    .clicked()
                                    {
                                        file_action = Some(false);
                                    }
                                    if widgets::button(
                                        ui,
                                        Some(Icon::File),
                                        "New NBT File",
                                        150.0,
                                        true,
                                    )
                                    .clicked()
                                    {
                                        file_action = Some(true);
                                    }
                                });
                        });
                }
            };
            if let Some(e) = edition_chooser(&mut child, "Select a World to Edit", Some(&mut more))
            {
                request = EditorRequest::PickWorld(e);
            }
            self.more_open = more_open && file_action.is_none();
            match file_action {
                Some(false) => {
                    if let Some(p) = settings.dialog("Open NBT File").pick_file() {
                        settings.visited(&p);
                        self.open_file(p, false);
                    }
                }
                Some(true) => {
                    if let Some(p) = settings
                        .dialog("New NBT File")
                        .set_file_name("new.dat")
                        .save_file()
                    {
                        settings.visited(&p);
                        self.open_file(p, true);
                    }
                }
                None => {}
            }
            if !self.status.is_empty() && self.workspaces.is_empty() {
                ui.painter().text(
                    full.center_bottom() - vec2(0.0, 24.0),
                    Align2::CENTER_CENTER,
                    &self.status,
                    theme::regular(13.0),
                    theme::TEXT_MUTED,
                );
            }
            return request;
        }
        self.left_panel(ui, settings);
        let full = ui.max_rect();
        let area = Rect::from_min_max(pos2(full.left() + LEFT_PANEL, full.top()), full.max);
        let full_precision = settings.full_precision;
        let status = self.status.clone();
        let Some(ws) = self.workspaces.get_mut(self.current) else {
            return request;
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        if ws.show_locator {
            locator(&mut child, ws, &mut self.status);
        } else {
            tabs(&mut child, ws, full_precision, settings, &mut self.status);
        }
        if !status.is_empty() {
            ui.painter().text(
                area.left_bottom() + vec2(12.0, -10.0),
                Align2::LEFT_BOTTOM,
                status,
                theme::regular(12.0),
                theme::TEXT_MUTED,
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
        child.set_clip_rect(panel.intersect(ui.clip_rect()));
        let ui = &mut child;
        // World selector and "+".
        ui.horizontal(|ui| {
            let names: Vec<(String, Icon)> = self
                .workspaces
                .iter()
                .map(|w| (w.name.clone(), w.edition.icon()))
                .collect();
            let (rect, resp) =
                ui.allocate_exact_size(vec2(LEFT_PANEL - 50.0, 30.0), Sense::click());
            ui.painter().rect_filled(
                rect,
                theme::radius(4),
                if resp.hovered() {
                    theme::BUTTON_HOVER
                } else {
                    theme::BUTTON
                },
            );
            if let Some((name, icon)) = names.get(self.current) {
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), Vec2::splat(16.0)),
                    *icon,
                    theme::TEXT_MUTED,
                );
                let font = theme::regular(14.0);
                let name = widgets::elide(ui, name, &font, rect.width() - 52.0);
                ui.painter().text(
                    rect.left_center() + vec2(28.0, 0.0),
                    Align2::LEFT_CENTER,
                    name,
                    theme::regular(14.0),
                    theme::TEXT,
                );
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.right_center() - vec2(12.0, 0.0), Vec2::splat(10.0)),
                Icon::ChevronDown,
                theme::TEXT_MUTED,
            );
            let popup = ui.make_persistent_id("world-switch");
            if resp.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            egui::popup_below_widget(
                ui,
                popup,
                &resp,
                egui::PopupCloseBehavior::CloseOnClick,
                |ui| {
                    ui.set_min_width(rect.width());
                    for (i, (name, _)) in names.iter().enumerate() {
                        if ui.selectable_label(i == self.current, name).clicked() {
                            self.current = i;
                            self.choosing = false;
                        }
                    }
                },
            );
            if widgets::button(ui, Some(Icon::Plus), "", 32.0, true)
                .on_hover_text("Open another world")
                .clicked()
            {
                self.choosing = true;
            }
        });
        ui.add_space(4.0);
        let half = (LEFT_PANEL - 20.0) / 2.0;
        let dirty = self.workspaces.get(self.current).is_some_and(|w| w.dirty());
        ui.horizontal(|ui| {
            if widgets::button(ui, Some(Icon::Close), "Close", half, true).clicked() {
                if dirty {
                    self.confirm_close = true;
                } else {
                    self.close_current();
                }
            }
            let save_label = if dirty { "Save*" } else { "Save" };
            if widgets::button(ui, Some(Icon::Save), save_label, half, true).clicked() {
                if let Some(ws) = self.workspaces.get_mut(self.current) {
                    self.status = match ws.save() {
                        Ok(s) => s,
                        Err(e) => format!("Save failed: {e}"),
                    };
                    crate::jobs::finished();
                }
            }
        });
        if self.confirm_close {
            let mut answer = None;
            egui::Modal::new(egui::Id::new("confirm-close")).show(ui.ctx(), |ui| {
                ui.label("This world has unsaved changes. Close it anyway?");
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
                    self.close_current();
                }
            }
        }
        let Some(ws) = self.workspaces.get_mut(self.current) else {
            return;
        };
        ui.add_space(4.0);
        ui.painter().line_segment(
            [
                pos2(panel.left(), ui.cursor().top()),
                pos2(panel.right(), ui.cursor().top()),
            ],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        ui.add_space(6.0);
        if ws.is_single_file() {
            return;
        }
        let big = |ui: &mut Ui, icon: Icon, text: &str, active: bool| -> bool {
            let (rect, resp) =
                ui.allocate_exact_size(vec2(LEFT_PANEL - 12.0, 40.0), Sense::click());
            ui.painter().rect_filled(
                rect,
                theme::radius(4),
                if resp.hovered() {
                    theme::BUTTON_HOVER
                } else {
                    Color32::from_rgb(0x26, 0x2B, 0x33)
                },
            );
            if active {
                ui.painter().rect_stroke(
                    rect,
                    theme::radius(4),
                    Stroke::new(1.2_f32, theme::BLUE),
                    egui::StrokeKind::Inside,
                );
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.left_center() + vec2(52.0, 0.0), Vec2::splat(22.0)),
                icon,
                theme::TEXT,
            );
            ui.painter().text(
                rect.left_center() + vec2(84.0, 0.0),
                Align2::LEFT_CENTER,
                text,
                theme::regular(16.0),
                theme::TEXT,
            );
            resp.clicked()
        };
        if big(ui, Icon::ChunkGrid, "Chunk Locator", ws.show_locator) {
            ws.show_locator = true;
        }
        ui.add_space(2.0);
        if big(ui, Icon::Gear, "World Settings", false) {
            let src = ws.level_source();
            if let Err(e) = ws.open_doc(src) {
                self.status = e;
            }
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (key, title) in [
                    ("players", "PLAYERS"),
                    ("maps", "MAPS"),
                    ("files", "ALL FILES"),
                ] {
                    let open = ws.expanded.contains(key);
                    if widgets::section_header(ui, title, open).clicked() {
                        if open {
                            ws.expanded.remove(key);
                        } else {
                            ws.expanded.insert(key.to_string());
                        }
                    }
                    if !open {
                        continue;
                    }
                    let mut clicked = None;
                    match key {
                        "players" => {
                            let img = settings
                                .extensions_dir()
                                .join("EntityInfo/images/Player.png");
                            let img = img.is_file().then_some(img);
                            for (label, src) in ws.players() {
                                if file_row_img(ui, 1, img.as_deref(), Icon::User, &label, None)
                                    .clicked()
                                {
                                    clicked = Some(src);
                                }
                            }
                        }
                        "maps" => {
                            let img = icons::pack_texture("item/filled_map.png");
                            for (label, src) in ws.maps() {
                                if file_row_img(ui, 1, img.as_deref(), Icon::File, &label, None)
                                    .clicked()
                                {
                                    clicked = Some(src);
                                }
                            }
                        }
                        _ => clicked = all_files(ui, ws),
                    }
                    if let Some(src) = clicked {
                        if let Err(e) = ws.open_doc(src) {
                            self.status = e;
                        }
                    }
                }
            });
    }

    fn close_current(&mut self) {
        if self.current < self.workspaces.len() {
            self.workspaces.remove(self.current);
            self.current = self.current.saturating_sub(1);
        }
    }
}

fn file_row(
    ui: &mut Ui,
    depth: usize,
    icon: Icon,
    label: &str,
    open: Option<bool>,
) -> egui::Response {
    file_row_img(ui, depth, None, icon, label, open)
}

/// A file row whose icon is an image file when `image` is given.
fn file_row_img(
    ui: &mut Ui,
    depth: usize,
    image: Option<&std::path::Path>,
    icon: Icon,
    label: &str,
    open: Option<bool>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::CARD);
    }
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    if let Some(o) = open {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x, rect.center().y), Vec2::splat(9.0)),
            if o {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            theme::TEXT_MUTED,
        );
    }
    let color = if icon == Icon::Folder {
        theme::YELLOW
    } else {
        theme::TEXT
    };
    let ir = Rect::from_center_size(pos2(x + 16.0, rect.center().y), Vec2::splat(16.0));
    if !image.is_some_and(|p| icons::file_image(ui.painter(), ir, p)) {
        icons::paint(ui.painter(), ir, icon, color);
    }
    let font = theme::regular(14.0);
    let label = widgets::elide(ui, label, &font, rect.right() - x - 36.0);
    ui.painter().text(
        pos2(x + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::regular(14.0),
        theme::TEXT,
    );
    resp
}

/// The ALL FILES tree; returns a document to open.
fn all_files(ui: &mut Ui, ws: &mut Workspace) -> Option<DocSource> {
    if let Some(save) = &ws.console {
        let mut out = None;
        let names: Vec<String> = save
            .archive
            .entries
            .iter()
            .map(|e| e.name.clone())
            .collect();
        for name in names {
            if name.ends_with(".mcr") {
                let key = format!("console:{name}");
                let open = ws.expanded.contains(&key);
                if file_row(ui, 1, Icon::ChunkGrid, &name, Some(open)).clicked() {
                    if open {
                        ws.expanded.remove(&key);
                    } else {
                        ws.expanded.insert(key.clone());
                    }
                }
                if open {
                    for (x, z) in workspace::console_region_chunks(save, &name) {
                        if file_row(ui, 3, Icon::File, &format!("Chunk {x}, {z}"), None).clicked() {
                            out = Some(DocSource::ConsoleChunk {
                                entry: name.clone(),
                                x,
                                z,
                            });
                        }
                    }
                }
            } else if file_row(ui, 1, Icon::File, &name, None).clicked() {
                out = Some(DocSource::ConsoleEntry(name.clone()));
            }
        }
        return out;
    }
    let root = ws.path.clone();
    dir_rows(ui, ws, &root, 1)
}

fn dir_rows(ui: &mut Ui, ws: &mut Workspace, dir: &Path, depth: usize) -> Option<DocSource> {
    let mut entries: Vec<(String, PathBuf, bool)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().to_string(),
                e.path(),
                e.path().is_dir(),
            )
        })
        .collect();
    entries.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    let mut out = None;
    for (name, path, is_dir) in entries {
        let key = path.display().to_string();
        let region = name.ends_with(".mca");
        if is_dir || region {
            let open = ws.expanded.contains(&key);
            let icon = if is_dir {
                Icon::Folder
            } else {
                Icon::ChunkGrid
            };
            if file_row(ui, depth, icon, &name, Some(open)).clicked() {
                if open {
                    ws.expanded.remove(&key);
                } else {
                    ws.expanded.insert(key.clone());
                }
            }
            if open {
                if is_dir {
                    if let Some(s) = dir_rows(ui, ws, &path, depth + 1) {
                        out = Some(s);
                    }
                } else {
                    for (x, z) in workspace::region_chunks(&path) {
                        if file_row(ui, depth + 2, Icon::File, &format!("Chunk {x}, {z}"), None)
                            .clicked()
                        {
                            out = Some(DocSource::JavaChunk {
                                region: path.clone(),
                                x,
                                z,
                            });
                        }
                    }
                }
            }
        } else if file_row(ui, depth, Icon::File, &name, None).clicked() {
            out = Some(DocSource::File(path));
        }
    }
    out
}

/// Enabled `NBT_EDITOR_STYLE` extensions (UMT's Player Info, Entity Info, …).
fn style_extensions(settings: &Settings) -> Vec<mctk_lua::Extension> {
    crate::builtin::list(&settings.extensions_dir())
        .into_iter()
        .filter(|e| {
            e.manifest
                .extension_types
                .iter()
                .any(|t| t == "NBT_EDITOR_STYLE")
        })
        .filter(|e| !settings.disabled_extensions.contains(&e.id))
        .collect()
}

/// Re-runs style extensions on the active document once its edits have settled.
fn refresh_styles(ui: &Ui, ws: &mut Workspace, settings: &Settings) -> Option<String> {
    let ed = ws.style_edition();
    let d = ws.active.and_then(|i| ws.docs.get_mut(i))?;
    let now = ui.input(|i| i.time);
    if d.seen.0 != d.tree.generation {
        d.seen = (d.tree.generation, now);
    }
    if d.styled == Some(d.tree.generation) {
        return None;
    }
    if d.styled.is_some() && now - d.seen.1 < 0.4 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(400));
        return None;
    }
    let exts = style_extensions(settings);
    let (styles, errors) = mctk_lua::style::style_tree(&exts, &d.tree.root.tag, ed, d.file_type());
    d.tree.styles = styles
        .into_iter()
        .map(|(path, s)| {
            let path = path
                .into_iter()
                .map(|p| match p {
                    mctk_lua::style::PathSeg::Key(k) => crate::nbt_view::Seg::Key(k),
                    mctk_lua::style::PathSeg::Index(i) => crate::nbt_view::Seg::Index(i),
                })
                .collect();
            (path, s)
        })
        .collect();
    d.styled = Some(d.tree.generation);
    (!errors.is_empty()).then(|| errors.join("; "))
}

fn tabs(
    ui: &mut Ui,
    ws: &mut Workspace,
    full_precision: bool,
    settings: &Settings,
    status: &mut String,
) {
    if let Some(err) = refresh_styles(ui, ws, settings) {
        *status = format!("Style extension error: {err}");
    }
    let full = ui.max_rect();
    let strip = Rect::from_min_size(full.min, vec2(full.width(), 40.0));
    ui.painter().rect_filled(strip, 0.0, theme::CARD);
    let mut x = strip.left() + 4.0;
    let mut close = None;
    for (i, d) in ws.docs.iter().enumerate() {
        let title = if d.tree.dirty {
            format!("{}*", d.title)
        } else {
            d.title.clone()
        };
        let w = ui.fonts(|f| {
            f.layout_no_wrap(title.clone(), theme::bold(14.0), Color32::WHITE)
                .size()
                .x
        }) + 54.0;
        let r = Rect::from_min_size(pos2(x, strip.top() + 4.0), vec2(w, 36.0));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        let active = ws.active == Some(i);
        ui.painter().rect_filled(
            r,
            egui::CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            if active {
                theme::BG
            } else if resp.hovered() {
                theme::BUTTON
            } else {
                theme::CARD
            },
        );
        if active {
            ui.painter().line_segment(
                [r.left_top(), r.right_top()],
                Stroke::new(2.0_f32, theme::TAB_BLUE),
            );
        }
        let cr = Rect::from_center_size(r.left_center() + vec2(16.0, 0.0), Vec2::splat(16.0));
        let close_resp = ui.interact(cr, ui.id().with(("tab-close", i)), Sense::click());
        icons::paint(
            ui.painter(),
            cr.shrink(3.0),
            Icon::Close,
            if close_resp.hovered() {
                Color32::WHITE
            } else {
                theme::TEXT_MUTED
            },
        );
        ui.painter().text(
            r.left_center() + vec2(34.0, 0.0),
            Align2::LEFT_CENTER,
            title,
            theme::bold(14.0),
            if active {
                Color32::WHITE
            } else {
                theme::TEXT_MUTED
            },
        );
        if close_resp.clicked() {
            close = Some(i);
        } else if resp.clicked() {
            ws.active = Some(i);
        }
        x += w + 2.0;
    }
    if let Some(i) = close {
        ws.close_doc(i);
    }
    let body = Rect::from_min_max(pos2(full.left(), strip.bottom()), full.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    match ws.active.and_then(|i| ws.docs.get_mut(i)) {
        Some(d) => d
            .tree
            .ui(&mut child, full_precision, &settings.extensions_dir()),
        None => {
            child.painter().text(
                body.center(),
                Align2::CENTER_CENTER,
                "Open World Settings, a player, a map or any file on the left.",
                theme::regular(14.0),
                theme::TEXT_DIM,
            );
        }
    }
}

fn locator(ui: &mut Ui, ws: &mut Workspace, status: &mut String) {
    let full = ui.max_rect();
    let bar = Rect::from_min_size(full.min, vec2(full.width(), 38.0));
    ui.painter().rect_filled(bar, 0.0, theme::BG);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(6.0, 4.0))));
    child.horizontal_centered(|ui| {
        let sel = ws.map.selected;
        if widgets::button(ui, None, "Open Selected Chunk", 0.0, sel.is_some()).clicked() {
            if let Some((x, z)) = sel {
                match ws
                    .chunk_source(x, z)
                    .ok_or_else(|| "That chunk is not stored".to_string())
                    .and_then(|s| ws.open_doc(s))
                {
                    Ok(()) => {}
                    Err(e) => *status = e,
                }
            }
        }
        let text = match sel {
            Some((x, z)) => format!("Chunk {x}, {z}"),
            None => "No Chunk Selected".into(),
        };
        ui.label(egui::RichText::new(text).italics().color(theme::TEXT_MUTED));
    });
    let cr = Rect::from_center_size(bar.right_center() - vec2(20.0, 0.0), Vec2::splat(24.0));
    if ui
        .interact(cr, ui.id().with("loc-close"), Sense::click())
        .clicked()
    {
        ws.show_locator = false;
    }
    icons::paint(ui.painter(), cr.shrink(5.0), Icon::Close, theme::TEXT_MUTED);
    let body = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let frame = ws.map.show_map(&mut child, true);
    if frame.response.double_clicked() {
        if let Some((x, z)) = ws.map.selected {
            if let Some(s) = ws.chunk_source(x, z) {
                if let Err(e) = ws.open_doc(s) {
                    *status = e;
                }
            }
        }
    }
    ws.map.show_chrome(&mut child, &frame);
}
