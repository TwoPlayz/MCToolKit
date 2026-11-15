//! The NBT Editor page.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::ctx_menu;
use crate::heads;
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
    /// The "Save changes?" dialog is up.
    confirm_save: bool,
    /// A dialog with one OK button: icon, title and message.
    notice: Option<(Icon, String, String)>,
    pub status: String,
    /// The messages showing in the top-right corner, oldest first.
    toasts: crate::toasts::Toasts,
}

/// A row of buttons in the middle of the dialog; the index of the one clicked.
fn centered_buttons(ui: &mut Ui, buttons: &[(Option<Icon>, &str)]) -> Option<usize> {
    const WIDTH: f32 = 116.0;
    const GAP: f32 = 8.0;
    let total = buttons.len() as f32 * WIDTH + buttons.len().saturating_sub(1) as f32 * GAP;
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        for (i, (icon, label)) in buttons.iter().enumerate() {
            if widgets::button(ui, *icon, label, WIDTH, true).clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// A dialog with a centred title, message lines (`true` = dimmed) and a centred row of buttons.
/// `Some(Some(i))` when button `i` was picked (Enter picks the first when `enter_first`),
/// `Some(None)` when it was dismissed (Esc or a click outside), `None` while it is still up.
fn dialog(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    lines: &[(String, bool)],
    buttons: &[(Option<Icon>, &str)],
    enter_first: bool,
) -> Option<Option<usize>> {
    let mut picked = None;
    let shown = egui::Modal::new(egui::Id::new(id)).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(title).size(18.0).color(Color32::WHITE));
            ui.add_space(4.0);
            for (text, dim) in lines {
                let text = egui::RichText::new(text);
                ui.label(if *dim { text.color(theme::TEXT_MUTED) } else { text });
            }
            ui.add_space(10.0);
            picked = centered_buttons(ui, buttons);
            if enter_first && picked.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                picked = Some(0);
            }
        });
    });
    if picked.is_some() {
        Some(picked)
    } else if shown.should_close() {
        Some(None)
    } else {
        None
    }
}

/// The dialog shown after a save: success, nothing to do, or the error.
fn saved_notice(result: Result<String, String>, name: &str) -> (Icon, String, String) {
    match result {
        Ok(s) if s == "Nothing to save" => (
            Icon::Info,
            "Nothing to save".into(),
            "There were no changes to write.".into(),
        ),
        Ok(s) => (Icon::Check, "Save successful".into(), format!("{s} in {name}.")),
        Err(e) => (Icon::Close, "Save failed".into(), e),
    }
}

/// Whether two paths are the same file or folder, however they were spelled.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
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
                    if widgets::tile(ui, e.icon(), e.label(), vec2(120.0, 120.0), false, true, false)
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
    /// The worlds open in this page (not single NBT files).
    pub fn open_worlds(&self) -> Vec<(String, PathBuf)> {
        self.workspaces
            .iter()
            .filter(|w| !w.is_single_file())
            .map(|w| (w.edition.subject(&w.name), w.original.clone()))
            .collect()
    }

    /// Switches to the open workspace for `found` and says it was already open.
    fn already_open(&mut self, found: usize, what: &str) {
        self.current = found;
        self.choosing = false;
        self.notice = Some((
            Icon::Info,
            "Already open".into(),
            format!("{what} is already open in the NBT editor, so it was not opened again."),
        ));
    }

    pub fn open_world(&mut self, path: &Path, edition: UiEdition, settings: &Settings) {
        if let Some(i) = self
            .workspaces
            .iter()
            .position(|w| !w.is_single_file() && same_path(&w.original, path))
        {
            let name = self.workspaces[i].edition.subject(&self.workspaces[i].name);
            self.already_open(i, &name);
            return;
        }
        match Workspace::open(
            path,
            edition,
            settings,
            false,
            false,
        ) {
            Ok(ws) => {
                // Player names and heads are ready before the world shows.
                heads::prefetch(
                    &settings.working_dir().join("heads"),
                    ws.players().into_iter().map(|(label, _)| label),
                );
                self.status.clear();
                self.workspaces.push(ws);
                self.current = self.workspaces.len() - 1;
                self.choosing = false;
            }
            Err(e) => self.status = format!("Could not open the world: {e}"),
        }
    }

    fn open_file(&mut self, path: PathBuf, new: bool) {
        if let Some(i) = self.workspaces.iter().position(|w| {
            w.is_single_file()
                && w.docs
                    .iter()
                    .any(|d| matches!(&d.source, DocSource::File(p) if same_path(p, &path)))
        }) {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            self.already_open(i, &name);
            return;
        }
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
                    tree.mark_unsaved();
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

    /// A message with an OK button, in the colour of its icon.
    fn show_notice(&mut self, ctx: &egui::Context) {
        let Some((icon, title, body)) = self.notice.clone() else {
            return;
        };
        let colour = match icon {
            Icon::Check => Color32::from_rgb(0x4C, 0xAF, 0x50),
            Icon::Close => Color32::from_rgb(0xE5, 0x57, 0x57),
            _ => theme::BLUE,
        };
        let mut done = false;
        let shown = egui::Modal::new(egui::Id::new("editor-notice")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 24.0, colour.gamma_multiply(0.18));
                icons::paint(ui.painter(), r.shrink(12.0), icon, colour);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&title).size(18.0).color(Color32::WHITE));
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&body).color(theme::TEXT_MUTED));
                ui.add_space(10.0);
                if centered_buttons(ui, &[(None, "OK")]).is_some()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    done = true;
                }
            });
        });
        if done || shown.should_close() {
            self.notice = None;
        }
    }

    /// Shows `status` as a message in the top-right corner (see `toasts`).
    fn show_toast(&mut self, ctx: &egui::Context) {
        if !self.status.is_empty() {
            let text = std::mem::take(&mut self.status);
            self.toasts.push(ctx, text);
        }
        self.toasts.show(ctx);
    }
    pub fn ui(&mut self, ui: &mut Ui, settings: &mut Settings) -> EditorRequest {
        let mut request = EditorRequest::None;
        heads::set_cache(settings.working_dir().join("heads"));
        self.show_notice(ui.ctx());
        self.show_toast(ui.ctx());
        if self.workspaces.is_empty() || self.choosing {
            // Shown across the whole page, as when no world is open yet.
            let area = ui.max_rect();
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
            if !self.workspaces.is_empty() {
                // Changed their mind: back to the world that was open.
                let r = Rect::from_min_size(area.left_top() + vec2(16.0, 16.0), vec2(110.0, 32.0));
                let mut back = ui.new_child(egui::UiBuilder::new().max_rect(r));
                if widgets::button(&mut back, Some(Icon::Close), "Cancel", 110.0, true).clicked() {
                    self.choosing = false;
                }
            }
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
            return request;
        }
        self.left_panel(ui, settings);
        let full = ui.max_rect();
        let area = Rect::from_min_max(pos2(full.left() + LEFT_PANEL, full.top()), full.max);
        let full_precision = settings.full_precision;
        let Some(ws) = self.workspaces.get_mut(self.current) else {
            return request;
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        if ws.show_locator {
            locator(&mut child, ws, &mut self.status);
        } else {
            tabs(&mut child, ws, full_precision, settings, &mut self.status);
        }
        request
    }

    fn left_panel(&mut self, ui: &mut Ui, settings: &mut Settings) {
        let full = ui.max_rect();
        let panel = Rect::from_min_size(full.min, vec2(LEFT_PANEL, full.height()));
        ui.painter().rect_filled(panel, 0.0, theme::BG);
        // Inside the panel's last pixel column, so the page beside it does not cover half of it.
        ui.painter().line_segment(
            [panel.right_top() - vec2(0.5, 0.0), panel.right_bottom() - vec2(0.5, 0.0)],
            Stroke::new(1.0_f32, theme::SEPARATOR),
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
                ui.allocate_exact_size(vec2(LEFT_PANEL - 12.0, 30.0), Sense::click());
            let popup = ui.make_persistent_id("world-switch");
            let open = widgets::dropdown_open(ui, popup);
            ui.painter().rect_filled(
                rect,
                widgets::dropdown_corners(open),
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
                if open { Icon::ChevronUp } else { Icon::ChevronDown },
                theme::TEXT_MUTED,
            );
            if resp.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            let current = self.current;
            let mut switch_to = None;
            let mut open_other = false;
            widgets::dropdown_pinned(
                ui,
                popup,
                &resp,
                theme::BUTTON,
                None,
                |ui, inner| {
                    for (i, (name, icon)) in names.iter().enumerate() {
                        let (row, resp) =
                            ui.allocate_exact_size(vec2(inner, 28.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(row, 0.0, theme::BUTTON_HOVER);
                        } else if i == current {
                            ui.painter().rect_filled(row, 0.0, Color32::from_white_alpha(14));
                        }
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(
                                row.left_center() + vec2(14.0, 0.0),
                                Vec2::splat(16.0),
                            ),
                            *icon,
                            theme::TEXT_MUTED,
                        );
                        let font = theme::regular(14.0);
                        let name = widgets::elide(ui, name, &font, row.width() - 40.0);
                        ui.painter().text(
                            row.left_center() + vec2(28.0, 0.0),
                            Align2::LEFT_CENTER,
                            name,
                            font,
                            theme::TEXT,
                        );
                        if resp.clicked() {
                            switch_to = Some(i);
                        }
                    }
                },
                28.0,
                |ui, inner| {
                    let (row, resp) =
                        ui.allocate_exact_size(vec2(inner, 28.0), Sense::click());
                    if resp.hovered() {
                        // The last row follows the list's rounded bottom corners.
                        let corners = egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 };
                        ui.painter().rect_filled(row, corners, theme::BUTTON_HOVER);
                    }
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(row.left_center() + vec2(14.0, 0.0), Vec2::splat(14.0)),
                        Icon::Plus,
                        theme::TEXT_MUTED,
                    );
                    ui.painter().text(
                        row.left_center() + vec2(28.0, 0.0),
                        Align2::LEFT_CENTER,
                        "Open another world",
                        theme::regular(14.0),
                        theme::TEXT,
                    );
                    if resp.clicked() {
                        open_other = true;
                    }
                },
            );
            if let Some(i) = switch_to {
                self.current = i;
                self.choosing = false;
            }
            if open_other {
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
            // Greyed out until there is something to write.
            if widgets::button(ui, Some(Icon::Save), "Save", half, dirty).clicked() {
                self.confirm_save = true;
            }
        });
        // Undo and Redo, under Close and Save: the open tab's tag edits and the file changes on
        // the left (deleting, renaming), whichever came last.
        ui.add_space(4.0);
        if let Some(ws) = self.workspaces.get_mut(self.current) {
            ws.sync_clock();
        }
        let (can_undo, can_redo) = self
            .workspaces
            .get(self.current)
            .map_or((false, false), |w| (w.can_undo_any(), w.can_redo_any()));
        ui.horizontal(|ui| {
            let undo = widgets::button(ui, Some(Icon::Undo), "Undo", half, can_undo)
                .on_hover_text(format!("Undo ({})", ctx_menu::shortcut("Z")));
            let redo = widgets::button(ui, Some(Icon::Redo), "Redo", half, can_redo).on_hover_text(
                format!("Redo ({} or {})", ctx_menu::shortcut("Y"), ctx_menu::shortcut("Shift+Z")),
            );
            if undo.clicked() || redo.clicked() {
                if let Some(ws) = self.workspaces.get_mut(self.current) {
                    let result = if undo.clicked() { ws.undo_any() } else { ws.redo_any() };
                    match result {
                        Ok(note) if !note.is_empty() => self.status = note,
                        Err(e) => self.status = e,
                        _ => {}
                    }
                }
            }
        });
        // Ctrl+Z and Ctrl+Y (Ctrl+Shift+Z) for the open tab, as the buttons above.
        let typing = self
            .workspaces
            .get(self.current)
            .filter(|w| !w.show_locator)
            .and_then(|w| w.active.and_then(|i| w.docs.get(i)))
            .is_none_or(|d| d.tree.is_typing());
        let (mut undo, mut redo) = (false, false);
        let idle = !typing && !self.confirm_save && !self.confirm_close && self.notice.is_none();
        if idle {
            use egui::{Key, Modifiers};
            ui.input_mut(|i| {
                redo |= i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y);
                undo |= i.consume_key(Modifiers::COMMAND, Key::Z);
            });
        }
        if undo || redo {
            if let Some(ws) = self.workspaces.get_mut(self.current) {
                let result = if redo && !undo { ws.redo_any() } else { ws.undo_any() };
                match result {
                    Ok(note) if !note.is_empty() => self.status = note,
                    Err(e) => self.status = e,
                    _ => {}
                }
            }
        }
        if self.confirm_save {
            let (name, copied) = self
                .workspaces
                .get(self.current)
                .map(|w| (w.name.clone(), w.copied))
                .unwrap_or_default();
            let backup = if copied {
                "The original is backed up to a _MCTBackup file before it is replaced."
            } else {
                "Each file is backed up the first time it is changed."
            };
            let answer = dialog(
                ui.ctx(),
                "confirm-save",
                "Save changes?",
                &[(format!("Your edits to {name} will be written to disk."), false), (backup.into(), true)],
                &[(Some(Icon::Save), "Save"), (None, "Cancel")],
                true,
            );
            match answer {
                Some(Some(0)) => {
                    self.confirm_save = false;
                    if let Some(ws) = self.workspaces.get_mut(self.current) {
                        let name = ws.name.clone();
                        let result = ws.save();
                        crate::jobs::finished();
                        self.notice = Some(saved_notice(result, &name));
                    }
                }
                Some(_) => self.confirm_save = false,
                None => {}
            }
        }
        if self.confirm_close {
            let answer = dialog(
                ui.ctx(),
                "confirm-close",
                "Close this world?",
                &[("It has unsaved changes.".into(), false)],
                &[(Some(Icon::Close), "Close anyway"), (None, "Cancel")],
                false,
            );
            if let Some(a) = answer {
                self.confirm_close = false;
                if a == Some(0) {
                    self.close_current();
                }
            }
        }
        self.close_tab_dialog(ui.ctx());
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
                    Stroke::new(1.2_f32, theme::accent()),
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
        ui.add_space(6.0);
        // The rows drawn last frame are what a range or drag selection works from.
        ws.file_rows_prev = std::mem::take(&mut ws.file_rows);
        ws.entry_rows_prev = std::mem::take(&mut ws.entry_rows);
        let rows_top = std::cell::Cell::new(egui::Pos2::ZERO);
        // Dragging picks rows (a box over them) instead of scrolling; the wheel and bar scroll.
        let scrolled = egui::ScrollArea::vertical()
            .auto_shrink(false)
            .drag_to_scroll(false)
            .show(ui, |ui| {
                rows_top.set(ui.max_rect().min);
                ui.spacing_mut().item_spacing.y = 0.0;
                for (key, title) in [
                    ("level", "WORLD SETTINGS"),
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
                        "level" => {
                            let src = ws.level_source();
                            let resp = file_row_img(ui, 1, None, Icon::Level, "level.dat", None);
                            match &src {
                                DocSource::File(p) => file_ctx(ui, ws, &resp, p, false),
                                other => entry_ctx(ui, ws, &resp, other),
                            }
                            if resp.clicked() {
                                clicked = Some(src);
                            }
                        }
                        "players" => {
                            let default = settings.info_dir().join("Player.png");
                            let default = default.is_file().then_some(default);
                            for (label, src) in ws.players() {
                                // A player's UUID is shown as their name and skin head once
                                // the profile service has been asked about it.
                                let profile = heads::profile(&settings.working_dir().join("heads"), &label, ui.ctx());
                                let img = profile.as_ref().and_then(|p| p.head.clone()).or_else(|| default.clone());
                                let shown = profile.as_ref().map_or(label.as_str(), |p| p.name.as_str());
                                let resp =
                                    file_row_img(ui, 1, img.as_deref(), Icon::User, shown, None);
                                match &src {
                                    DocSource::File(p) => file_ctx(ui, ws, &resp, p, false),
                                    other => entry_ctx(ui, ws, &resp, other),
                                }
                                if resp.clicked() {
                                    clicked = Some(src);
                                }
                            }
                        }
                        "maps" => {
                            let img = icons::pack_texture("item/filled_map.png");
                            for (label, src) in ws.maps() {
                                let resp =
                                    file_row_img(ui, 1, img.as_deref(), Icon::File, &label, None);
                                match &src {
                                    DocSource::File(p) => file_ctx(ui, ws, &resp, p, false),
                                    other => entry_ctx(ui, ws, &resp, other),
                                }
                                if resp.clicked() {
                                    clicked = Some(src);
                                }
                            }
                        }
                        _ => clicked = all_files(ui, ws, settings),
                    }
                    if let Some(src) = clicked {
                        if let Err(e) = ws.open_doc(src) {
                            self.status = e;
                        }
                    }
                }
            });
        file_select(ui, ws, scrolled.inner_rect, rows_top.get());
        sidebar_shortcuts(ui, ws);
        file_rename_dialog(ui, ws);
        // Deleting is reversible until the save (Undo on the row), so it asks for nothing.
        let doomed = std::mem::take(&mut ws.pending_delete);
        if !doomed.is_empty() {
            let note = match doomed.as_slice() {
                [one] => format!("Deleted {}", crate::workspace::source_name(one)),
                many => format!("Deleted {} items", many.len()),
            };
            ws.begin_batch();
            for source in doomed {
                ws.mark_deleted(source);
            }
            ws.end_batch();
            ws.file_note = note;
        }
        if !ws.file_note.is_empty() {
            self.status = std::mem::take(&mut ws.file_note);
        }
    }

    /// "Save changes to <tab>?" for a tab with unsaved edits that was closed.
    fn close_tab_dialog(&mut self, ctx: &egui::Context) {
        let Some(ws) = self.workspaces.get_mut(self.current) else {
            return;
        };
        let Some(uid) = ws.pending_close else {
            return;
        };
        let Some(i) = ws.docs.iter().position(|d| d.uid == uid) else {
            ws.pending_close = None;
            return;
        };
        let title = tab_title(&ws.docs[i]);
        let note = if ws.copied {
            "The world's working copy is written back to the original."
        } else {
            "The file is backed up the first time it is changed."
        };
        let answer = dialog(
            ctx,
            "confirm-close-tab",
            &format!("Save changes to {title}?"),
            &[
                ("Your edits will be lost if you don't save.".into(), false),
                (note.into(), true),
            ],
            &[(Some(Icon::Save), "Save"), (None, "Don't Save"), (None, "Cancel")],
            true,
        );
        match answer {
            Some(Some(0)) => {
                ws.pending_close = None;
                let result = ws.save_doc(i);
                crate::jobs::finished();
                let saved = result.is_ok();
                self.notice = Some(saved_notice(result, &title));
                if saved {
                    ws.close_doc(i);
                }
            }
            Some(Some(1)) => {
                ws.pending_close = None;
                ws.close_doc(i);
            }
            Some(_) => ws.pending_close = None,
            None => {}
        }
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
    let shown = widgets::elide(ui, label, &font, rect.right() - x - 36.0);
    let cut = shown != label;
    ui.painter().text(
        pos2(x + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        theme::regular(14.0),
        theme::TEXT,
    );
    // A name that did not fit is shown whole while the pointer is on it.
    if cut {
        resp.on_hover_text(label)
    } else {
        resp
    }
}

/// The ALL FILES tree; returns a document to open.
fn all_files(ui: &mut Ui, ws: &mut Workspace, settings: &Settings) -> Option<DocSource> {
    if ws.console.is_some() {
        let names: Vec<String> = ws
            .console
            .as_ref()
            .map(|s| s.archive.entries.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default();
        return console_rows(ui, ws, settings, &names, "", 1);
    }    let root = ws.path.clone();
    dir_rows(ui, ws, settings, &root, 1)
}

/// The entries of a console save under `prefix`, shown as folders like a Java world's.
fn console_rows(
    ui: &mut Ui,
    ws: &mut Workspace,
    settings: &Settings,
    names: &[String],
    prefix: &str,
    depth: usize,
) -> Option<DocSource> {
    let mut folders: Vec<String> = Vec::new();
    let mut files: Vec<&String> = Vec::new();
    for name in names {
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        match rest.split_once('/') {
            Some((dir, _)) => {
                if !folders.iter().any(|f| f == dir) {
                    folders.push(dir.to_string());
                }
            }
            None => files.push(name),
        }
    }
    folders.sort_by_key(|f| f.to_lowercase());
    files.sort_by_key(|f| f.to_lowercase());
    let mut out = None;
    for dir in folders {
        let path = format!("{prefix}{dir}/");
        let key = format!("console-dir:{path}");
        let open = ws.expanded.contains(&key);
        let resp = file_row(ui, depth, Icon::Folder, &dir, Some(open));
        if resp.clicked() {
            if open {
                ws.expanded.remove(&key);
            } else {
                ws.expanded.insert(key);
            }
        }
        if open {
            if let Some(s) = console_rows(ui, ws, settings, names, &path, depth + 1) {
                out = Some(s);
            }
        }
    }
    for name in files {
        let base = name.rsplit('/').next().unwrap_or(name);
        if name.ends_with(".mcr") {
            let key = format!("console:{name}");
            let open = ws.expanded.contains(&key);
            let resp = file_row(ui, depth, Icon::ChunkGrid, base, Some(open));
            entry_ctx(ui, ws, &resp, &DocSource::ConsoleEntry(name.clone()));
            if resp.clicked() {
                if open {
                    ws.expanded.remove(&key);
                } else {
                    ws.expanded.insert(key.clone());
                }
            }
            if open {
                let chunks = ws
                    .console
                    .as_ref()
                    .map(|s| workspace::console_region_chunks(s, name))
                    .unwrap_or_default();
                for (x, z) in chunks {
                    if file_row(ui, depth + 2, Icon::File, &format!("Chunk {x}, {z}"), None)
                        .clicked()
                    {
                        out = Some(DocSource::ConsoleChunk { entry: name.clone(), x, z });
                    }
                }
            }
        } else {
            let (img, icon) = kind_icon(ui, settings, name, name.starts_with("players/"));
            let resp = file_row_img(ui, depth, img.as_deref(), icon, base, None);
            let source = DocSource::ConsoleEntry(name.clone());
            entry_ctx(ui, ws, &resp, &source);
            if resp.clicked() {
                out = Some(source);
            }
        }
    }
    out
}

/// The icon of a file in ALL FILES: a map or a player gets the picture used in its own section.
fn kind_icon(
    ui: &Ui,
    settings: &Settings,
    name: &str,
    in_players: bool,
) -> (Option<PathBuf>, Icon) {
    let base = name.rsplit('/').next().unwrap_or(name);
    if base == "level.dat" || base == "level.dat_old" {
        return (None, Icon::Level);
    }
    if base.starts_with("map_") && base.ends_with(".dat") {
        return (icons::pack_texture("item/filled_map.png"), Icon::File);
    }
    if in_players || base.starts_with("P_") {
        let stem = base.split('.').next().unwrap_or(base);
        let head = heads::profile(&settings.working_dir().join("heads"), stem, ui.ctx())
            .and_then(|p| p.head);
        let img = settings.info_dir().join("Player.png");
        return (head.or_else(|| img.is_file().then_some(img)), Icon::User);
    }
    (None, Icon::File)
}

fn dir_rows(
    ui: &mut Ui,
    ws: &mut Workspace,
    settings: &Settings,
    dir: &Path,
    depth: usize,
) -> Option<DocSource> {
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
            let resp = file_row(ui, depth, icon, &name, Some(open));
            file_ctx(ui, ws, &resp, &path, is_dir);
            if resp.clicked() {
                if open {
                    ws.expanded.remove(&key);
                } else {
                    ws.expanded.insert(key.clone());
                }
            }
            if open {
                if is_dir {
                    if let Some(s) = dir_rows(ui, ws, settings, &path, depth + 1) {
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
        } else {
            // A player's data and its backup are listed under their name, with their head,
            // once those are known.
            let (img, icon) = kind_icon(ui, settings, &name, is_player_file(&name));
            let shown = player_label(&name).unwrap_or_else(|| name.clone());
            let resp = file_row_img(ui, depth, img.as_deref(), icon, &shown, None);
            file_ctx(ui, ws, &resp, &path, false);
            if resp.clicked() {
                out = Some(DocSource::File(path));
            }
        }
    }
    out
}

/// A bar on the left of a side-panel row: red and the row faded when it is deleted but still
/// waiting for the save, green when it is new since the last save.
fn row_marker(ui: &Ui, rect: Rect, gone: bool, fresh: bool, edited: bool) {
    if !gone && !fresh && !edited {
        return;
    }
    if gone {
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgba_premultiplied(34, 14, 16, 150));
    }
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
        0.0,
        if gone {
            theme::RED
        } else if fresh {
            theme::EDITOR_ACCENT
        } else {
            crate::nbt_view::EDITED
        },
    );
}

/// The right-click menu of a deleted row: it can only be brought back.
fn restore_menu(ws: &mut Workspace, resp: &egui::Response, source: &DocSource) {
    let root = ws.gone_root(source);
    resp.context_menu(|ui| {
        if root.is_some() && crate::nbt_view::white_item(ui, Icon::Undo, "Undo") {
            if let Some(root) = &root {
                ws.restore_deleted(root);
                ws.file_note = format!("Restored {}", crate::workspace::source_name(root));
            }
        }
    });
}

/// A click on a side-panel row: Ctrl adds or removes it, Shift takes everything from the last
/// pick, a right-click inside the selection leaves it alone, anything else picks just this row.
fn pick_file(ws: &mut Workspace, path: &Path, command: bool, shift: bool, secondary: bool) {
    let inside = ws.selected_files.iter().any(|p| p == path);
    if secondary && inside {
        return;
    }
    if command && !secondary {
        if let Some(i) = ws.selected_files.iter().position(|p| p == path) {
            ws.selected_files.remove(i);
            ws.selected_file = ws.selected_files.last().cloned();
        } else {
            ws.selected_files.push(path.to_path_buf());
            ws.selected_file = Some(path.to_path_buf());
        }
        return;
    }
    if shift && !secondary {
        let at = |p: &Path| ws.file_rows_prev.iter().position(|(q, _)| q == p);
        if let (Some(a), Some(b)) = (ws.selected_file.as_deref().and_then(at), at(path)) {
            let (lo, hi) = (a.min(b), a.max(b));
            ws.selected_files = ws.file_rows_prev[lo..=hi].iter().map(|(p, _)| p.clone()).collect();
            return;
        }
    }
    ws.selected_files = vec![path.to_path_buf()];
    ws.selected_file = Some(path.to_path_buf());
}

/// Starts renaming a file or folder, with its name ready to be replaced.
fn start_rename(ws: &mut Workspace, path: &Path) {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    ws.renaming_file = Some((path.to_path_buf(), name));
}

/// Selection, highlight and right-click menu (cut, copy, paste, rename, delete) of a file or
/// folder row in the side panel. The menu acts on the whole selection when the row is in it.
fn file_ctx(ui: &mut Ui, ws: &mut Workspace, resp: &egui::Response, path: &Path, is_dir: bool) {
    ws.file_rows.push((path.to_path_buf(), resp.rect));
    if resp.clicked() || resp.secondary_clicked() {
        let (command, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        pick_file(ws, path, command, shift, resp.secondary_clicked());
        ws.selected_entry = None;
        ws.selected_entries.clear();
    }
    let source = DocSource::File(path.to_path_buf());
    let gone = ws.is_gone(&source);
    if resp.double_clicked() && !gone {
        start_rename(ws, path);
    }
    if ws.selected_files.iter().any(|p| p == path) || ws.selected_file.as_deref() == Some(path) {
        ui.painter().rect_filled(
            resp.rect,
            0.0,
            theme::accent_alpha(40),
        );
    }
    let fresh = !gone && ws.is_fresh(path);
    row_marker(ui, resp.rect, gone, fresh, !gone && !fresh && ws.is_edited(path));
    if gone {
        restore_menu(ws, resp, &source);
        return;
    }
    let targets: Vec<PathBuf> = if ws.selected_files.iter().any(|p| p == path) {
        ws.selected_files.clone()
    } else {
        vec![path.to_path_buf()]
    };
    let (can_undo, can_redo) = (ws.file_can_undo(path), ws.file_can_redo(path));
    resp.context_menu(|ui| {
        if can_undo && crate::nbt_view::white_item(ui, Icon::Undo, "Undo") {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            ws.file_note = match ws.file_undo(path) {
                Ok(()) => format!("Undid the last change to {name}"),
                Err(e) => e,
            };
        }
        if can_redo && crate::nbt_view::white_item(ui, Icon::Redo, "Redo") {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            ws.file_note = match ws.file_redo(path) {
                Ok(()) => format!("Redid the change to {name}"),
                Err(e) => e,
            };
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with("map_") && name.ends_with(".dat") && ctx_menu::item(ui, Icon::File, "Open Map Editor", "", true) {
            match ws.open_doc(DocSource::File(path.to_path_buf())) {
                Ok(()) => {
                    let opened = ws.active.and_then(|i| ws.docs.get_mut(i)).is_some_and(|d| d.tree.open_map_editor());
                    if !opened {
                        ws.file_note = "That file has no map picture".into();
                    }
                }
                Err(e) => ws.file_note = e,
            }
        }
        if ctx_menu::item(ui, Icon::Cut, "Cut", &ctx_menu::shortcut("X"), true) {
            ws.clip_files(&targets, true);
        }
        if ctx_menu::item(ui, Icon::Copy, "Copy", &ctx_menu::shortcut("C"), true) {
            ws.clip_files(&targets, false);
        }
        let can_paste = ws.file_clip.is_some();
        if ctx_menu::item(ui, Icon::Paste, "Paste", &ctx_menu::shortcut("V"), can_paste) {
            paste_beside(ws, path, is_dir);
        }
        if targets.len() == 1 && ctx_menu::item(ui, Icon::Rename, "Rename", "Double-click", true) {
            start_rename(ws, path);
        }
        ui.separator();
        if ctx_menu::item(ui, Icon::Trash, "Delete", "Del", true) {
            ws.pending_delete = targets.iter().cloned().map(DocSource::File).collect();
        }
    });
}

/// Drag-select over the side panel's rows, as in a file manager.
fn file_select(ui: &Ui, ws: &mut Workspace, area: Rect, top: egui::Pos2) {
    let (pressed, down, pos, command) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
            i.modifiers.command,
        )
    });
    // Leave the scrollbar on the right alone.
    let inside = Rect::from_min_max(area.min, pos2(area.right() - 14.0, area.bottom()));
    if pressed {
        ws.file_marquee = None;
        if let Some(p) = pos.filter(|p| inside.contains(*p) && ui.rect_contains_pointer(inside)) {
            ws.file_marquee = Some(workspace::FileMarquee {
                start: p - top,
                base: if command { ws.selected_files.clone() } else { Vec::new() },
                base_entries: if command { ws.selected_entries.clone() } else { Vec::new() },
                active: false,
            });
        }
    }
    if !down {
        ws.file_marquee = None;
        return;
    }
    let (Some(mut m), Some(cur)) = (ws.file_marquee.take(), pos) else {
        return;
    };
    let start = top + m.start;
    if !m.active && start.distance(cur) > 5.0 {
        m.active = true;
    }
    if m.active {
        let rect = Rect::from_two_pos(start, cur);
        let mut picked = m.base.clone();
        for (p, _) in ws.file_rows.iter().filter(|(_, r)| r.intersects(rect)) {
            if !picked.contains(p) {
                picked.push(p.clone());
            }
        }
        if let Some(first) = ws.file_rows.iter().find(|(_, r)| r.intersects(rect)) {
            ws.selected_file = Some(first.0.clone());
        }
        ws.selected_files = picked;
        let mut entries = m.base_entries.clone();
        for (s, _) in ws.entry_rows.iter().filter(|(_, r)| r.intersects(rect)) {
            if !entries.contains(s) {
                entries.push(s.clone());
            }
        }
        if let Some((s, _)) = ws.entry_rows.iter().find(|(_, r)| r.intersects(rect)) {
            ws.selected_entry = Some(s.clone());
        }
        ws.selected_entries = entries;
        ui.painter().with_clip_rect(area).rect(
            rect,
            0.0,
            theme::accent_alpha(40),
            Stroke::new(1.0_f32, theme::accent()),
            egui::StrokeKind::Inside,
        );
        ui.ctx().request_repaint();
    }
    ws.file_marquee = Some(m);
}

/// The "Rename" window for a file or folder: the old name starts out selected.
fn file_rename_dialog(ui: &Ui, ws: &mut Workspace) {
    let Some((path, mut name)) = ws.renaming_file.take() else {
        return;
    };
    let ctx = ui.ctx().clone();
    let (mut keep, mut done) = (true, false);
    egui::Window::new("Rename")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(&ctx, |ui| {
            let field = egui::Id::new("file-rename-field");
            let mut edit = egui::TextEdit::singleline(&mut name).id(field).show(ui);
            edit.response.request_focus();
            let seen = egui::Id::new("file-rename-seen");
            let key = path.display().to_string();
            if ui.data(|d| d.get_temp::<String>(seen)).as_deref() != Some(key.as_str()) {
                ui.data_mut(|d| d.insert_temp(seen, key));
                // Without the extension, the name part is what is selected, as in a file manager.
                let len = name.chars().count();
                let stem = if path.is_dir() {
                    len
                } else {
                    name.rfind('.').filter(|i| *i > 0).map_or(len, |i| name[..i].chars().count())
                };
                edit.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(stem),
                )));
                edit.state.store(ui.ctx(), field);
                ui.ctx().request_repaint();
            }
            ui.horizontal(|ui| {
                done = ui.button("Rename").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter));
                keep = !ui.button("Cancel").clicked() && !ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
    if done || !keep {
        ctx.data_mut(|d| d.remove::<String>(egui::Id::new("file-rename-seen")));
    }
    if done {
        match ws.rename_file(&path, &name) {
            Ok(note) if !note.is_empty() => ws.file_note = note,
            Ok(_) => {}
            Err(e) => ws.file_note = e,
        }
    } else if keep {
        ws.renaming_file = Some((path, name));
    }
}
/// Selection, highlight and right-click menu of a row that is not a file on disk (a Bedrock
/// database record or a console archive entry); those can only be deleted.
fn entry_ctx(ui: &mut Ui, ws: &mut Workspace, resp: &egui::Response, source: &DocSource) {
    ws.entry_rows.push((source.clone(), resp.rect));
    if resp.clicked() || resp.secondary_clicked() {
        let (command, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let inside = ws.selected_entries.contains(source);
        if resp.secondary_clicked() && inside {
            // The menu then acts on everything picked.
        } else if resp.clicked() && command {
            if let Some(i) = ws.selected_entries.iter().position(|s| s == source) {
                ws.selected_entries.remove(i);
            } else {
                ws.selected_entries.push(source.clone());
            }
        } else if resp.clicked() && shift && ws.selected_entry.is_some() {
            let at = |s: &DocSource| ws.entry_rows_prev.iter().position(|(q, _)| q == s);
            if let (Some(a), Some(b)) = (ws.selected_entry.as_ref().and_then(at), at(source)) {
                let (lo, hi) = (a.min(b), a.max(b));
                ws.selected_entries = ws.entry_rows_prev[lo..=hi].iter().map(|(s, _)| s.clone()).collect();
            }
        } else {
            ws.selected_entries = vec![source.clone()];
        }
        ws.selected_entry = ws.selected_entries.last().cloned().or_else(|| Some(source.clone()));
        ws.selected_file = None;
        ws.selected_files.clear();
    }
    if ws.selected_entries.contains(source) || ws.selected_entry.as_ref() == Some(source) {
        ui.painter().rect_filled(
            resp.rect,
            0.0,
            theme::accent_alpha(40),
        );
    }
    let gone = ws.is_gone(source);
    row_marker(ui, resp.rect, gone, false, false);
    if gone {
        restore_menu(ws, resp, source);
        return;
    }
    let is_map = match source {
        DocSource::ConsoleEntry(n) => {
            let base = n.rsplit('/').next().unwrap_or(n);
            base.starts_with("map_") && base.ends_with(".dat")
        }
        DocSource::BedrockKey(k) => k.starts_with(b"map_"),
        _ => false,
    };
    resp.context_menu(|ui| {
        if is_map && ctx_menu::item(ui, Icon::File, "Open Map Editor", "", true) {
            match ws.open_doc(source.clone()) {
                Ok(()) => {
                    let opened = ws.active.and_then(|i| ws.docs.get_mut(i)).is_some_and(|d| d.tree.open_map_editor());
                    if !opened {
                        ws.file_note = "That entry has no map picture".into();
                    }
                }
                Err(e) => ws.file_note = e,
            }
        }
        if is_map {
            ui.separator();
        }
        if ctx_menu::item(ui, Icon::Trash, "Delete", "Del", true) {
            ws.pending_delete = if ws.selected_entries.contains(source) {
                ws.selected_entries.clone()
            } else {
                vec![source.clone()]
            };
        }
    });
}

/// Pastes into `path` when it is a folder, else into the folder holding it.
fn paste_beside(ws: &mut Workspace, path: &Path, is_dir: bool) {
    let dir = if is_dir {
        Some(path.to_path_buf())
    } else {
        path.parent().map(Path::to_path_buf)
    };
    if let Some(dir) = dir {
        ws.file_note = ws.paste_file(&dir).unwrap_or_else(|e| e);
    }
}

/// Ctrl+C / X / V / A and Delete on the files picked in the side panel.
fn sidebar_shortcuts(ui: &Ui, ws: &mut Workspace) {
    if !ws.pending_delete.is_empty()
        || ws.renaming_file.is_some()
        || ui.ctx().wants_keyboard_input()
        || !ui.ui_contains_pointer()
    {
        return;
    }
    let Some(sel) = ws.selected_file.clone().or_else(|| ws.selected_files.first().cloned()) else {
        // Rows that are not files (console entries, Bedrock records): select all and delete.
        if ws.selected_entry.is_some() || !ws.selected_entries.is_empty() {
            let (delete, all) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Delete),
                    i.modifiers.command && i.key_pressed(egui::Key::A),
                )
            });
            if all {
                ws.selected_entries = ws.entry_rows_prev.iter().map(|(s, _)| s.clone()).collect();
            }
            if delete {
                ws.pending_delete = if ws.selected_entries.is_empty() {
                    ws.selected_entry.clone().into_iter().collect()
                } else {
                    ws.selected_entries.clone()
                };
            }
        }
        return;
    };
    let (mut copy, mut cut, mut paste, mut delete, mut all) = (false, false, false, false, false);
    ui.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Copy => copy = true,
                egui::Event::Cut => cut = true,
                egui::Event::Paste(_) => paste = true,
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    egui::Key::C if modifiers.command => copy = true,
                    egui::Key::X if modifiers.command => cut = true,
                    egui::Key::V if modifiers.command => paste = true,
                    egui::Key::A if modifiers.command => all = true,
                    egui::Key::Delete => delete = true,
                    _ => {}
                },
                _ => {}
            }
        }
    });
    let targets: Vec<PathBuf> = if ws.selected_files.is_empty() {
        vec![sel.clone()]
    } else {
        ws.selected_files.clone()
    };
    if all {
        // Everything beside the picked row.
        let parent = sel.parent().map(Path::to_path_buf);
        ws.selected_files = ws
            .file_rows_prev
            .iter()
            .filter(|(p, _)| p.parent().map(Path::to_path_buf) == parent)
            .map(|(p, _)| p.clone())
            .collect();
    }
    if cut {
        ws.clip_files(&targets, true);
    } else if copy {
        ws.clip_files(&targets, false);
    }
    if paste && ws.file_clip.is_some() {
        let is_dir = sel.is_dir();
        paste_beside(ws, &sel, is_dir);
    }
    if delete {
        ws.pending_delete = targets.into_iter().map(DocSource::File).collect();
    }
}
/// Works out the notes beside the active document's tags once its edits have settled.
fn refresh_styles(ui: &Ui, ws: &mut Workspace) {
    let ed = ws.style_edition();
    let Some(d) = ws.active.and_then(|i| ws.docs.get_mut(i)) else { return };
    let now = ui.input(|i| i.time);
    if d.seen.0 != d.tree.generation {
        d.seen = (d.tree.generation, now);
    }
    if d.styled == Some(d.tree.generation) {
        return;
    }
    if d.styled.is_some() && now - d.seen.1 < 0.4 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(400));
        return;
    }
    d.tree.styles = crate::info::style_tree(&d.tree.root.tag, ed, d.file_type());
    d.styled = Some(d.tree.generation);
}
/// Whether a file is named after a player's UUID, as the files kept for a player are.
fn is_player_file(file: &str) -> bool {
    file.split_once('.').is_some_and(|(stem, ext)| {
        heads::normalise(stem).is_some() && matches!(ext, "dat" | "dat_old")
    })
}

/// A player's file as it is listed: their name, with what kind of file it is when that is not
/// their data itself. `None` until the name is known, or for other files.
fn player_label(file: &str) -> Option<String> {
    let (stem, ext) = file.split_once('.')?;
    let name = heads::cached_name(stem)?;
    let kind = match ext {
        "dat" => " (current)",
        "dat_old" => " (backup)",
        _ => return None,
    };
    Some(format!("{name}{kind}"))
}

/// The icon a tab shows: the one its file has in the side panel (a player's head, the level
/// icon, a picture's own thumbnail, ...).
fn tab_icon(ui: &Ui, settings: &Settings, d: &workspace::Doc) -> (Option<PathBuf>, Icon) {
    match &d.source {
        DocSource::File(p) => {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            kind_icon(ui, settings, name, is_player_file(name))
        }
        DocSource::ConsoleEntry(name) => kind_icon(ui, settings, name, name.starts_with("players/")),
        DocSource::BedrockKey(k) => {
            let name = String::from_utf8_lossy(k).to_string();
            if name.starts_with("map_") {
                (icons::pack_texture("item/filled_map.png"), Icon::File)
            } else if name == "~local_player" || name.starts_with("player_") {
                kind_icon(ui, settings, &name, true)
            } else {
                (None, Icon::File)
            }
        }
        DocSource::JavaChunk { .. } | DocSource::BedrockChunk { .. } | DocSource::ConsoleChunk { .. } => {
            (None, Icon::ChunkGrid)
        }
    }
}

/// What a tab is called: a player's file shows their name once it is known.
fn tab_title(d: &workspace::Doc) -> String {
    if let DocSource::File(p) = &d.source {
        let file = p.file_name().and_then(|n| n.to_str());
        if let Some(label) = file.and_then(player_label) {
            return label;
        }
    }
    d.title.clone()
}

/// A small X that lights up under the pointer, as on the messages in the corner.
pub fn close_button(ui: &mut Ui, rect: Rect, id: egui::Id) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, theme::radius(4), Color32::from_white_alpha(28));
    }
    icons::paint(
        ui.painter(),
        rect.shrink(rect.width() * 0.25),
        Icon::Close,
        if resp.hovered() { Color32::WHITE } else { theme::TEXT_MUTED },
    );
    resp
}

fn tabs(
    ui: &mut Ui,
    ws: &mut Workspace,
    full_precision: bool,
    settings: &Settings,
    status: &mut String,
) {
    refresh_styles(ui, ws);
    let full = ui.max_rect();
    let strip = Rect::from_min_size(full.min, vec2(full.width(), 40.0));
    ui.painter().rect_filled(strip, 0.0, theme::CARD);
    let ctx = ui.ctx().clone();
    let mut close = None;
    // The tab being dragged (its uid) and where inside it the pointer took hold; kept in egui's
    // memory so it survives the frames in between, and dropped when the button is released.
    let drag_id = egui::Id::new("editor-tab-drag");
    let held: Option<(u64, f32)> = ctx
        .data(|m| m.get_temp(drag_id))
        .filter(|_| ctx.input(|i| i.pointer.primary_down()));
    if held.is_none() {
        ctx.data_mut(|m| m.remove::<(u64, f32)>(drag_id));
    }
    let pointer_x = ctx.input(|i| i.pointer.interact_pos()).map(|p| p.x);
    let titles: Vec<String> = ws
        .docs
        .iter()
        .map(|d| {
            let title = tab_title(d);
            if d.tree.dirty {
                format!("{title}*")
            } else {
                title
            }
        })
        .collect();
    let widths: Vec<f32> = titles
        .iter()
        .map(|t| {
            ui.fonts(|f| {
                f.layout_no_wrap(t.clone(), theme::bold(14.0), Color32::WHITE)
                    .size()
                    .x
            }) + 78.0
        })
        .collect();
    // Where each tab sits when nothing is being dragged.
    let mut slots = Vec::with_capacity(widths.len());
    let mut at = strip.left() + 4.0;
    for w in &widths {
        slots.push(at);
        at += w + 2.0;
    }
    let total = (at - strip.left() - 4.0 - 2.0).max(0.0);
    // When the tabs do not all fit, the strip scrolls and two arrows take the right-hand end.
    let arrow = 24.0;
    let overflow = total > strip.width() - 8.0;
    let view = if overflow {
        Rect::from_min_max(strip.min, pos2(strip.right() - 2.0 * arrow - 4.0, strip.bottom()))
    } else {
        strip
    };
    let view_w = view.width() - 8.0;
    let max_scroll = (total - view_w).max(0.0);
    let held_at = held.and_then(|(uid, _)| ws.docs.iter().position(|d| d.uid == uid));
    let active_uid = ws.active.and_then(|i| ws.docs.get(i)).map(|d| d.uid);
    // Bring the current tab into view when it changes or a tab is added.
    if (active_uid, ws.docs.len()) != ws.tab_seen && held.is_none() {
        if let Some(i) = ws.active.filter(|i| *i < widths.len()) {
            let left = slots[i] - strip.left() - 4.0;
            let right = left + widths[i];
            if left < ws.tab_scroll {
                ws.tab_scroll = left;
            } else if right > ws.tab_scroll + view_w {
                ws.tab_scroll = right - view_w;
            }
        }
        ws.tab_seen = (active_uid, ws.docs.len());
    }
    if overflow && ui.rect_contains_pointer(strip) {
        let wheel = ctx.input(|i| i.smooth_scroll_delta);
        ws.tab_scroll -= wheel.x + wheel.y;
    }
    // Holding a tab near either end of the strip scrolls it, faster the closer to the end.
    if let (true, Some(px)) = (held.is_some() && overflow, pointer_x) {
        let zone = 48.0;
        // Not capped at the strip's edge: the further past it the pointer is, the faster.
        let push = ((zone - (px - view.left())) / zone).clamp(0.0, 8.0)
            - ((zone - (view.right() - px)) / zone).clamp(0.0, 8.0);
        if push != 0.0 {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            ws.tab_scroll -= push * 700.0 * dt;
            ctx.request_repaint();
        }
    }
    ws.tab_scroll = ws.tab_scroll.clamp(0.0, max_scroll);
    let off =ctx.animate_value_with_time(egui::Id::new("tab-scroll"), ws.tab_scroll, if held.is_some() { 0.0 } else { 0.12 });
    let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(view).id_salt("tab-strip"));
    tui.set_clip_rect(view);
    // The dragged tab is drawn last, over the others.
    let order: Vec<usize> = (0..ws.docs.len())
        .filter(|i| Some(*i) != held_at)
        .chain(held_at)
        .collect();
    for i in order {
        let d = &ws.docs[i];
        let title = titles[i].clone();
        let w = widths[i];
        let is_held = held_at == Some(i);
        let target_x = match (held, pointer_x) {
            // Free to go past either end of the strip; it settles into its place on release.
            (Some((_, grab)), Some(px)) if is_held => px - grab,
            _ => slots[i] - off,
        };
        // The others slide to their new places; the dragged tab follows the pointer at once.
        let x = ctx.animate_value_with_time(
            egui::Id::new(("tab-x", d.uid)),
            target_x,
            if is_held { 0.0 } else { 0.12 },
        );
        let r = Rect::from_min_size(pos2(x, strip.top() + 4.0), vec2(w, 36.0));
        // Keyed by the tab, not its position, so a drag keeps hold of it while tabs reorder.
        let resp = tui
            .interact(r, tui.id().with(("tab", d.uid)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.drag_started() {
            ws.active = Some(i);
            if let Some(px) = pointer_x {
                ctx.data_mut(|m| m.insert_temp(drag_id, (d.uid, px - r.left())));
            }
        }
        let active = ws.active == Some(i);
        tui.painter().rect_filled(
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
            tui.painter().line_segment(
                [r.left_top(), r.right_top()],
                Stroke::new(2.0_f32, theme::accent()),
            );
        }
        let cr = Rect::from_center_size(r.left_center() + vec2(17.0, 0.0), Vec2::splat(24.0));
        let id = tui.id().with(("tab-close", d.uid));
        let close_resp = close_button(&mut tui, cr, id);
        // The same icon the file has in the side panel, between the X and the name.
        let ir = Rect::from_center_size(r.left_center() + vec2(45.0, 0.0), Vec2::splat(16.0));
        let (picture, icon) = tab_icon(&tui, settings, d);
        if !picture.as_deref().is_some_and(|p| icons::file_image(tui.painter(), ir, p)) {
            icons::paint(
                tui.painter(),
                ir,
                icon,
                if active { Color32::WHITE } else { theme::TEXT_MUTED },
            );
        }
        tui.painter().text(
            r.left_center() + vec2(60.0, 0.0),
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
    }
    if overflow {
        for (n, (glyph, step)) in [("\u{2039}", -180.0_f32), ("\u{203A}", 180.0)].into_iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(strip.right() - 2.0 * arrow - 2.0 + n as f32 * arrow, strip.top() + 4.0),
                vec2(arrow, 36.0),
            );
            let resp = ui.interact(r, ui.id().with(("tab-arrow", n)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, theme::BUTTON);
            }
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                glyph,
                theme::bold(18.0),
                theme::TEXT,
            );
            if resp.clicked() {
                ws.tab_scroll = (ws.tab_scroll + step).clamp(0.0, max_scroll);
            }
        }
    }
    if let Some(i) = close {
        // Unsaved edits: ask first (the dialog is in the left panel code, next to the others).
        if ws.docs[i].tree.dirty && !ws.docs[i].tree.read_only {
            ws.pending_close = Some(ws.docs[i].uid);
        } else {
            ws.close_doc(i);
        }
    } else if let (Some(i), Some(_), Some(px)) = (held_at, held, pointer_x) {
        // The dragged tab goes after every other tab whose middle is left of the pointer.
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        // The pointer decides, not the dragged tab's own middle: a wide tab's middle can never
        // get left of a narrow first tab's.
        let middle = px;
        let mut left = strip.left() + 4.0 - off;
        let mut target = 0;
        for (j, w) in widths.iter().enumerate() {
            if j != i {
                target += (middle > left + w / 2.0) as usize;
                left += w + 2.0;
            }
        }
        if target != i {
            let doc = ws.docs.remove(i);
            ws.docs.insert(target, doc);
            ws.active = Some(target);
        }
    }
    let body = Rect::from_min_max(pos2(full.left(), strip.bottom()), full.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let mut map_request = None;
    match ws.active.and_then(|i| ws.docs.get_mut(i)) {
        Some(d) => {
            d.tree.deletable = matches!(
                d.source,
                DocSource::File(_) | DocSource::BedrockKey(_) | DocSource::ConsoleEntry(_)
            ) && !d.tree.read_only;
            d.tree.ui(&mut child, full_precision, &settings.info_dir());
            if !d.tree.status.is_empty() {
                *status = std::mem::take(&mut d.tree.status);
            }
            if std::mem::take(&mut d.tree.delete_requested) {
                ws.pending_delete = vec![d.source.clone()];
            }
            map_request = d.tree.open_map_request.take();
        }
        None => {
            child.painter().text(
                body.center(),
                Align2::CENTER_CENTER,
                "Open level.dat, a player, a map or any file on the left.",
                theme::regular(14.0),
                theme::TEXT_DIM,
            );
        }
    }
    // A map item asked for its map: the file in this world named after it.
    if let Some(name) = map_request {
        let found = ws.maps().into_iter().find(|(n, _)| n.trim_end_matches(".dat") == name);
        match found {
            Some((_, source)) => match ws.open_doc(source) {
                Ok(()) => {
                    let opened = ws.active.and_then(|i| ws.docs.get_mut(i)).is_some_and(|d| d.tree.open_map_editor());
                    if !opened {
                        *status = "That map has no picture".into();
                    }
                }
                Err(e) => *status = e,
            },
            None => *status = format!("This world has no {name} file"),
        }
    }
}

fn locator(ui: &mut Ui, ws: &mut Workspace, status: &mut String) {
    let full = ui.max_rect();
    let bar = Rect::from_min_size(full.min, vec2(full.width(), 38.0));
    ui.painter().rect_filled(bar, 0.0, theme::BG);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(6.0, 4.0))));
    child.horizontal_centered(|ui| {
        let sel = ws.map.selected.clone();
        if widgets::green_button(ui, None, "Open Selected Chunks", 0.0, !sel.is_empty()).clicked() {
            open_chunks(ws, &sel, status);
        }
        let text = match sel.as_slice() {
            [] => "No Chunk Selected".to_string(),
            [(x, z)] => format!("Chunk {x}, {z}"),
            many => format!("{} Chunks Selected", many.len()),
        };
        ui.label(egui::RichText::new(text).italics().color(theme::TEXT_MUTED));
    });
    let cr = Rect::from_center_size(bar.right_center() - vec2(20.0, 0.0), Vec2::splat(28.0));
    let id = ui.id().with("loc-close");
    if close_button(ui, cr, id).clicked() {
        ws.show_locator = false;
    }
    let body = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    ws.map.selectable = true;
    let frame = ws.map.show_map(&mut child, true);
    if frame.response.double_clicked() && !child.input(|i| i.modifiers.command) {
        let sel = ws.map.selected.clone();
        open_chunks(ws, &sel, status);
    }
    ws.map.show_chrome(&mut child, &frame);
}

/// Opens each of the chunks in its own tab; the last one is left showing.
fn open_chunks(ws: &mut Workspace, chunks: &[(i32, i32)], status: &mut String) {
    let mut missing = 0;
    for &(x, z) in chunks {
        match ws.chunk_source(x, z) {
            Some(s) => {
                if let Err(e) = ws.open_doc(s) {
                    *status = e;
                }
            }
            None => missing += 1,
        }
    }
    if missing > 0 {
        *status = if chunks.len() == 1 {
            "That chunk is not stored".to_string()
        } else {
            format!("{missing} of the chunks are not stored")
        };
    }
}
