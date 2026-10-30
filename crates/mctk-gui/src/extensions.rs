//! The Extensions page: the built-in and imported extensions as cards, with an On/Off switch,
//! details, and (for imported ones) a remove button. "Import from file" adds a `.zip` or a
//! folder's `manifest.json`.

use eframe::egui::{self, Align2, Color32, Rect, RichText, Stroke, Ui, Vec2, pos2, vec2};

use crate::icons::{self, Icon};
use crate::settings::Settings;
use crate::theme;
use crate::widgets;

#[derive(Default)]
pub struct ExtensionsPage {
    info: Option<String>,
    /// An imported extension waiting for the user to confirm its removal.
    confirm_remove: Option<String>,
    pub status: String,
}

impl ExtensionsPage {
    pub fn ui(&mut self, ui: &mut Ui, settings: &mut Settings) {
        let full = ui.max_rect();
        let strip = Rect::from_min_size(full.min, vec2(full.width(), 46.0));
        ui.painter().rect_filled(strip, 0.0, theme::BG);
        ui.painter().text(
            strip.left_center() + vec2(20.0, 2.0),
            Align2::LEFT_CENTER,
            "Extensions",
            theme::bold(15.0),
            Color32::WHITE,
        );
        let mut imported = false;
        let mut at = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(strip.right() - 190.0, strip.top() + 7.0),
            vec2(170.0, 32.0),
        )));
        if widgets::button(&mut at, Some(Icon::Plus), "Import from file", 170.0, true).clicked() {
            imported = true;
        }
        if imported {
            self.import(settings);
        }
        let body = Rect::from_min_max(
            pos2(full.left() + 10.0, strip.bottom() + 8.0),
            full.max - vec2(10.0, 8.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let ui = &mut child;
        let list = crate::builtin::list(&settings.extensions_dir());
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for ext in &list {
                    self.card(ui, settings, ext);
                    ui.add_space(10.0);
                }
            });
        if let Some(id) = self.info.clone() {
            match list.iter().find(|e| e.id == id) {
                Some(ext) => self.info_dialog(ui.ctx(), ext),
                None => self.info = None,
            }
        }
        if let Some(id) = self.confirm_remove.clone() {
            let name = list
                .iter()
                .find(|e| e.id == id)
                .map_or(id.clone(), |e| e.manifest.name.clone());
            self.remove_dialog(ui.ctx(), settings, &id, &name);
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
    }

    /// Asks for a `.zip` (or a folder's `manifest.json`) and installs it.
    fn import(&mut self, settings: &Settings) {
        let Some(path) = settings
            .dialog("Import an extension")
            .add_filter("Extension (.zip or manifest.json)", &["zip", "json"])
            .pick_file()
        else {
            return;
        };
        let dir = settings.extensions_dir();
        let _ = std::fs::create_dir_all(&dir);
        let zip = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let result = if zip {
            mctk_lua::install_zip(&path, &dir)
        } else {
            match path.parent() {
                Some(folder) => mctk_lua::install_folder(folder, &dir),
                None => return,
            }
        };
        self.status = match result {
            Ok(ext) if crate::builtin::is_builtin(&ext.id) => {
                // The name is taken by a built-in one: put it back as shipped.
                crate::builtin::restore(&dir);
                format!(
                    "\"{}\" has the same name as a built-in extension; rename it in its manifest.json",
                    ext.manifest.name
                )
            }
            Ok(ext) => format!("Imported {}", ext.manifest.name),
            Err(e) => format!("Could not import the extension: {e}"),
        };
    }

    fn remove_dialog(&mut self, ctx: &egui::Context, settings: &mut Settings, id: &str, name: &str) {
        let (mut yes, mut no) = (false, false);
        egui::Modal::new(egui::Id::new("ext-remove")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                RichText::new(format!("Remove {name}?"))
                    .font(theme::bold(18.0))
                    .color(Color32::WHITE),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("The extension and its files are deleted from MCToolKit. The original you imported is not touched.")
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                yes = widgets::button(ui, Some(Icon::Trash), "Remove", 0.0, true).clicked();
                no = widgets::button(ui, None, "Cancel", 0.0, true).clicked();
            });
        });
        if yes {
            self.status = match mctk_lua::remove(&settings.extensions_dir(), id) {
                Ok(()) => {
                    settings.disabled_extensions.retain(|d| d != id);
                    settings.save();
                    format!("Removed {name}")
                }
                Err(e) => format!("Could not remove {name}: {e}"),
            };
        }
        if yes || no {
            self.confirm_remove = None;
        }
    }

    fn card(&mut self, ui: &mut Ui, settings: &mut Settings, ext: &mctk_lua::Extension) {
        let m = &ext.manifest;
        let enabled = !settings.disabled_extensions.contains(&ext.id);
        let imported = !crate::builtin::is_builtin(&ext.id);
        let width = ui.available_width() - 6.0;
        widgets::card(ui, 16, |ui| {
            ui.set_width(width - 32.0);
            ui.set_height(106.0);
            let r = ui.max_rect();
            let p = ui.painter().clone();
            // Thumbnail.
            let icon = Rect::from_min_size(r.min, Vec2::splat(64.0));
            if !icons::file_image(&p, icon, &ext.dir.join("thumbnail.png")) {
                p.rect_filled(icon, theme::radius(4), theme::CHIP);
                icons::paint(&p, icon.shrink(16.0), Icon::Extensions, theme::TEXT_MUTED);
            }
            let ver = Rect::from_min_size(pos2(r.left(), r.top() + 76.0), vec2(64.0, 26.0));
            p.rect_stroke(
                ver,
                theme::radius(3),
                Stroke::new(1.0_f32, theme::CHIP),
                egui::StrokeKind::Inside,
            );
            p.text(
                ver.center(),
                Align2::CENTER_CENTER,
                &m.version,
                theme::regular(13.0),
                theme::TEXT_MUTED,
            );
            // Name and description.
            p.text(
                pos2(r.left() + 84.0, r.top() + 14.0),
                Align2::LEFT_CENTER,
                &m.name,
                theme::regular(16.5),
                Color32::WHITE,
            );
            let desc = p.layout(
                m.description.clone(),
                theme::regular(13.5),
                theme::TEXT_MUTED,
                r.width() - 420.0,
            );
            p.galley(
                pos2(r.left() + 84.0, r.top() + 30.0),
                desc,
                theme::TEXT_MUTED,
            );
            // Author chip and type tags.
            let mut x = r.left() + 84.0;
            let y = r.top() + 89.0;
            if !m.author.is_empty() {
                let w = p
                    .layout_no_wrap(m.author.clone(), theme::regular(13.5), theme::TEXT)
                    .size()
                    .x
                    + 34.0;
                let chip = Rect::from_min_size(pos2(x, y - 12.0), vec2(w, 24.0));
                p.rect_filled(chip, theme::radius(12), theme::CHIP);
                icons::paint(
                    &p,
                    Rect::from_center_size(chip.left_center() + vec2(13.0, 0.0), Vec2::splat(12.0)),
                    Icon::User,
                    theme::TEXT,
                );
                p.text(
                    chip.left_center() + vec2(24.0, 0.0),
                    Align2::LEFT_CENTER,
                    &m.author,
                    theme::regular(13.5),
                    theme::TEXT,
                );
                x += w + 14.0;
            }
            for t in &m.extension_types {
                let label = t
                    .replace("NBT_EDITOR_STYLE", "STYLE")
                    .replace("WORLD_TOOL", "WORLD TOOL")
                    .replace('_', " ");
                icons::paint(
                    &p,
                    Rect::from_center_size(pos2(x + 8.0, y), Vec2::splat(15.0)),
                    Icon::AppEditor,
                    theme::EDITOR_ACCENT,
                );
                let w = p
                    .layout_no_wrap(label.clone(), theme::bold(13.0), theme::GREEN)
                    .size()
                    .x;
                p.text(
                    pos2(x + 20.0, y),
                    Align2::LEFT_CENTER,
                    label,
                    theme::bold(13.0),
                    theme::GREEN,
                );
                x += w + 36.0;
            }
            // Editions.
            let ex = r.right() - 250.0;
            for (i, e) in ["JAVA", "BEDROCK", "CONSOLE"].iter().enumerate() {
                let yy = r.top() + 8.0 + i as f32 * 21.0;
                let ok = m.supports(e);
                icons::paint(
                    &p,
                    Rect::from_center_size(pos2(ex, yy), Vec2::splat(14.0)),
                    if ok { Icon::Check } else { Icon::Cross },
                    if ok { theme::GREEN } else { theme::TEXT_DIM },
                );
                p.text(
                    pos2(ex + 18.0, yy),
                    Align2::LEFT_CENTER,
                    *e,
                    theme::regular(13.5),
                    theme::TEXT_MUTED,
                );
            }
            p.line_segment(
                [
                    pos2(r.right() - 132.0, r.top()),
                    pos2(r.right() - 132.0, r.bottom() - 30.0),
                ],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            let more = Rect::from_min_size(pos2(ex - 6.0, r.top() + 78.0), vec2(98.0, 26.0));
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt((&ext.id, "info"))
                    .max_rect(more),
            );
            if widgets::button(&mut child, Some(Icon::Info), "More Info", 98.0, true).clicked() {
                self.info = Some(ext.id.clone());
            }
            if imported {
                let rm = Rect::from_min_size(pos2(r.right() - 34.0, r.top() + 76.0), Vec2::splat(28.0));
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt((&ext.id, "remove"))
                        .max_rect(rm),
                );
                if widgets::icon_button(&mut child, Icon::Trash, 28.0, "Remove this extension").clicked() {
                    self.confirm_remove = Some(ext.id.clone());
                }
            }
            let tr = Rect::from_min_size(pos2(r.right() - 108.0, r.top()), vec2(106.0, 30.0));
            // Each card needs its own id, or the On/Off segments clash between cards.
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt((&ext.id, "toggle"))
                    .max_rect(tr),
            );
            let mut on = (!enabled) as usize;
            if widgets::segmented(&mut child, &mut on, &["On", "Off"], 106.0, true) {
                if on == 1 {
                    settings.disabled_extensions.push(ext.id.clone());
                } else {
                    settings.disabled_extensions.retain(|d| d != &ext.id);
                }
                settings.save();
            }
        });
    }

    fn info_dialog(&mut self, ctx: &egui::Context, ext: &mctk_lua::Extension) {
        let m = &ext.manifest;
        // The licence ships with each extension (BSD 2-Clause asks for it to be reproduced).
        let licence = ["LICENSE", "LICENCE"]
            .iter()
            .find_map(|f| std::fs::read_to_string(ext.dir.join(f)).ok())
            .unwrap_or_default();
        let mut close = false;
        egui::Modal::new(egui::Id::new("ext-info")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(
                RichText::new(&m.name)
                    .font(theme::bold(20.0))
                    .color(Color32::WHITE),
            );
            ui.label(RichText::new(&m.description).color(theme::TEXT_MUTED));
            ui.add_space(8.0);
            egui::Grid::new("ext-info-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    for (k, v) in [
                        ("Version", &m.version),
                        ("Author", &m.author),
                        ("Link", &m.link),
                    ] {
                        if !v.is_empty() {
                            ui.label(RichText::new(k).color(theme::TEXT_MUTED));
                            if k == "Link" {
                                ui.hyperlink(v);
                            } else {
                                ui.label(v);
                            }
                            ui.end_row();
                        }
                    }
                });
            if !licence.is_empty() {
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(licence.trim())
                                .size(12.0)
                                .color(theme::TEXT_DIM),
                        );
                    });
            }
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        if close {
            self.info = None;
        }
    }
}
