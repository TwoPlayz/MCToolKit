//! "Open a … World" dialog: installed worlds sorted by last played or name, plus Browse.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::icons::{self, Icon};
use crate::settings::Settings;
use crate::theme;
use crate::widgets;
use crate::world::Edition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEdition {
    Java,
    Bedrock,
    Xbox360,
    Ps3,
    WiiU,
}

impl UiEdition {
    pub const ALL: [UiEdition; 5] = [
        UiEdition::Java,
        UiEdition::Bedrock,
        UiEdition::Xbox360,
        UiEdition::Ps3,
        UiEdition::WiiU,
    ];

    pub fn label(self) -> &'static str {
        match self {
            UiEdition::Java => "Java",
            UiEdition::Bedrock => "Bedrock",
            UiEdition::Xbox360 => "Xbox 360",
            UiEdition::Ps3 => "PlayStation 3",
            UiEdition::WiiU => "Wii U",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            UiEdition::Java => Icon::Java,
            UiEdition::Bedrock => Icon::Bedrock,
            UiEdition::Xbox360 => Icon::Xbox360,
            UiEdition::Ps3 => Icon::PlayStation3,
            UiEdition::WiiU => Icon::WiiU,
        }
    }

    pub fn world(self) -> Edition {
        match self {
            UiEdition::Java => Edition::Java,
            UiEdition::Bedrock => Edition::Bedrock,
            _ => Edition::Console,
        }
    }
}

struct Entry {
    name: String,
    path: PathBuf,
    last_played: i64,
    icon: Option<PathBuf>,
}

pub enum Picked {
    Nothing,
    Cancelled,
    Open(PathBuf),
}

pub struct WorldPicker {
    pub edition: UiEdition,
    entries: Vec<Entry>,
    by_name: bool,
    selected: Option<usize>,
    browsed: Option<Entry>,
    error: Option<String>,
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Name, last-played time and icon of a world folder, if it is one.
fn describe(path: &Path, edition: UiEdition) -> Option<Entry> {
    match edition {
        UiEdition::Java => {
            let level = path.join("level.dat");
            if !level.is_file() {
                return None;
            }
            let root = mctk_nbt::read_file(&level, mctk_nbt::Flavor::Java)
                .ok()
                .map(|(_, r)| r);
            let data = root.as_ref().and_then(|r| r.tag.get("Data"));
            let name = data
                .and_then(|d| d.get("LevelName"))
                .and_then(mctk_nbt::Tag::as_str)
                .map(str::to_string);
            let played = data
                .and_then(|d| d.get("LastPlayed"))
                .and_then(mctk_nbt::Tag::as_i64)
                .map(|ms| ms / 1000);
            Some(Entry {
                name: name.unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                }),
                path: path.to_path_buf(),
                last_played: played.unwrap_or_else(|| mtime(&level)),
                icon: Some(path.join("icon.png")),
            })
        }
        UiEdition::Bedrock => {
            if !path.join("db").is_dir() && path.extension().is_none_or(|e| e != "mcworld") {
                return None;
            }
            let name = std::fs::read_to_string(path.join("levelname.txt"))
                .ok()
                .map(|s| s.trim().to_string());
            let played = std::fs::read(path.join("level.dat"))
                .ok()
                .and_then(|b| mctk_bedrock::LevelDat::parse(&b).ok())
                .and_then(|l| l.root.tag.get("LastPlayed").and_then(mctk_nbt::Tag::as_i64));
            Some(Entry {
                name: name.unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                }),
                path: path.to_path_buf(),
                last_played: played.unwrap_or_else(|| mtime(path)),
                icon: Some(path.join("world_icon.jpeg")),
            })
        }
        _ => {
            if !path.is_file() {
                return None;
            }
            // A PS3 GAMEDATA file is named after, and shows the icon of, its save folder; an
            // Xbox 360 savegame.dat is named after its folder.
            let folder = path.parent().filter(|_| path.ends_with("GAMEDATA"));
            let generic = path
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("savegame.dat"));
            let shown = folder
                .or_else(|| path.parent().filter(|_| generic))
                .unwrap_or(path);
            Some(Entry {
                name: shown
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: path.to_path_buf(),
                last_played: mtime(path),
                icon: folder.map(|f| f.join("ICON0.PNG")),
            })
        }
    }
}

/// Whether a file found while scanning a console worlds folder is a save of that console.
fn console_candidate(path: &Path, edition: UiEdition) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match edition {
        UiEdition::Ps3 => name == "gamedata",
        UiEdition::Xbox360 => {
            // savegame.dat: compressed length, 4 bytes, decompressed length (big-endian),
            // then the stream. Checking the header skips other .dat files (level.dat, …).
            if ext != "dat" {
                return false;
            }
            let Ok(mut f) = std::fs::File::open(path) else {
                return false;
            };
            let mut h = [0u8; 12];
            if std::io::Read::read_exact(&mut f, &mut h).is_err() {
                return false;
            }
            let len = f.metadata().map_or(0, |m| m.len());
            let compressed = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
            let decompressed = u32::from_be_bytes([h[8], h[9], h[10], h[11]]) as u64;
            compressed > 0 && compressed + 12 <= len && decompressed >= compressed
        }
        _ => ext == "ext" || ext == "wii",
    }
}

/// Console saves under `dir` and its subfolders (a bounded walk).
fn scan_console(dir: &Path, edition: UiEdition, out: &mut Vec<PathBuf>) {
    let mut stack = vec![(dir.to_path_buf(), 0)];
    let mut visited = 0;
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            visited += 1;
            if visited > 20_000 {
                return;
            }
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && depth < 8 {
                stack.push((p, depth + 1));
            } else if t.is_file() && console_candidate(&p, edition) {
                out.push(p);
            }
        }
    }
}

impl WorldPicker {
    pub fn new(edition: UiEdition) -> Self {
        WorldPicker {
            edition,
            entries: Vec::new(),
            by_name: false,
            selected: None,
            browsed: None,
            error: None,
        }
    }

    fn folder(&self, settings: &Settings) -> Option<PathBuf> {
        let f = match self.edition {
            UiEdition::Java => &settings.java_worlds,
            UiEdition::Bedrock => &settings.bedrock_worlds,
            UiEdition::WiiU => &settings.wiiu_worlds,
            UiEdition::Xbox360 => &settings.xbox_worlds,
            UiEdition::Ps3 => &settings.ps3_worlds,
        };
        (!f.is_empty()).then(|| PathBuf::from(f))
    }

    pub fn refresh(&mut self, ctx: &egui::Context, settings: &Settings) {
        self.entries.clear();
        self.selected = None;
        let _ = ctx;
        if let Some(dir) = self.folder(settings) {
            let paths: Vec<PathBuf> = match self.edition {
                UiEdition::Java | UiEdition::Bedrock => std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .collect(),
                console => {
                    let mut v = Vec::new();
                    scan_console(&dir, console, &mut v);
                    v
                }
            };
            self.entries
                .extend(paths.iter().filter_map(|p| describe(p, self.edition)));
        }
        self.sort();
    }

    fn sort(&mut self) {
        if self.by_name {
            self.entries.sort_by_key(|e| e.name.to_lowercase());
        } else {
            self.entries
                .sort_by_key(|e| std::cmp::Reverse(e.last_played));
        }
    }

    fn row(ui: &mut egui::Ui, e: &Entry, selected: bool, height: f32) -> egui::Response {
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, 0.0, Color32::from_rgb(0x26, 0x3A, 0x55));
        } else if resp.hovered() {
            p.rect_filled(rect, 0.0, theme::BUTTON);
        }
        let icon = Rect::from_min_size(rect.min + vec2(8.0, 6.0), Vec2::splat(height - 12.0));
        if !e
            .icon
            .as_deref()
            .is_some_and(|i| icons::file_image(p, icon, i))
            // Minecraft's pack.png, as UMT shows for worlds without an image.
            && !icons::pack_file("pack.png").is_some_and(|i| icons::file_image(p, icon, &i))
        {
            p.rect_filled(icon, theme::radius(3), theme::CHIP);
        }
        let x = icon.right() + 14.0;
        p.text(
            pos2(x, rect.center().y - 9.0),
            Align2::LEFT_CENTER,
            &e.name,
            theme::bold(15.0),
            theme::TEXT,
        );
        let path = e.path.display().to_string().replace('\\', "/");
        let short = if path.chars().count() > 40 {
            let tail: String = path
                .chars()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{}...{}", path.chars().take(17).collect::<String>(), tail)
        } else {
            path
        };
        p.text(
            pos2(x, rect.center().y + 11.0),
            Align2::LEFT_CENTER,
            short,
            theme::regular(12.5),
            theme::TEXT_MUTED,
        );
        resp
    }

    /// Draws the dialog; returns what the user did this frame.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Picked {
        let mut result = Picked::Nothing;
        let size = vec2(520.0, 650.0f32.min(ctx.screen_rect().height() - 40.0));
        let modal = egui::Modal::new(egui::Id::new("world-picker")).frame(
            egui::Frame::new()
                .fill(theme::CARD)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(theme::radius(6)),
        );
        let response = modal.show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            // Title bar.
            let (bar, _) = ui.allocate_exact_size(vec2(size.x, 38.0), Sense::hover());
            ui.painter().text(
                bar.left_center() + vec2(16.0, 0.0),
                Align2::LEFT_CENTER,
                format!("Open a {} World", self.edition.label()),
                theme::regular(16.0),
                theme::TEXT,
            );
            let close =
                Rect::from_center_size(bar.right_center() - vec2(20.0, 0.0), Vec2::splat(26.0));
            let cr = ui.interact(close, egui::Id::new("picker-close"), Sense::click());
            icons::paint(
                ui.painter(),
                close.shrink(7.0),
                Icon::Close,
                if cr.hovered() {
                    Color32::WHITE
                } else {
                    theme::TEXT
                },
            );
            if cr.clicked() {
                result = Picked::Cancelled;
            }
            // Sort / folder / refresh row.
            ui.horizontal(|ui| {
                ui.add_space(30.0);
                ui.label(egui::RichText::new("Sort by:").color(theme::TEXT));
                let mut sort = self.by_name as usize;
                if widgets::segmented(ui, &mut sort, &["Last Played", "Name"], 0.0, true) {
                    self.by_name = sort == 1;
                    self.sort();
                }
                let folder = self.folder(settings);
                let console = self.edition.world() == Edition::Console;
                // Console saves have no install location: the button chooses the folder
                // that is searched (with its subfolders) instead of opening it.
                let (label, enabled) = if console {
                    ("World Folder...", true)
                } else {
                    ("Minecraft Folder", folder.is_some())
                };
                let resp = widgets::button(ui, Some(Icon::Folder), label, 0.0, enabled);
                let resp = match (&folder, console) {
                    (Some(f), true) => resp.on_hover_text(f.display().to_string()),
                    _ => resp,
                };
                if resp.clicked() {
                    if console {
                        let mut d = rfd::FileDialog::new()
                            .set_title(format!("Select {} Worlds Folder", self.edition.label()));
                        if let Some(f) = folder.as_ref().filter(|f| f.is_dir()) {
                            d = d.set_directory(f);
                        }
                        if let Some(f) = d.pick_folder() {
                            let f = f.display().to_string();
                            match self.edition {
                                UiEdition::Xbox360 => settings.xbox_worlds = f,
                                UiEdition::Ps3 => settings.ps3_worlds = f,
                                _ => settings.wiiu_worlds = f,
                            }
                            settings.save();
                            self.refresh(ctx, settings);
                        }
                    } else if let Some(f) = folder {
                        let _ = std::process::Command::new("explorer").arg(f).spawn();
                    }
                }
                if widgets::button(ui, Some(Icon::Refresh), "Refresh", 0.0, true).clicked() {
                    self.refresh(ctx, settings);
                }
            });
            ui.add_space(6.0);
            let list_h = size.y - 38.0 - 44.0 - 108.0 - 86.0;
            egui::Frame::new()
                .fill(theme::BG)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .inner_margin(egui::Margin::same(1))
                .show(ui, |ui| {
                    ui.set_height(list_h);
                    ui.set_width(size.x - 10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            if self.entries.is_empty() {
                                ui.add_space(list_h / 2.0 - 10.0);
                                ui.vertical_centered(|ui| {
                                    let console = self.edition.world() == Edition::Console;
                                    let text = match (console, self.folder(settings)) {
                                        (true, None) => {
                                            "Choose a World Folder to list its saves, or Browse."
                                        }
                                        (true, Some(_)) => {
                                            "No saves found in the world folder or its subfolders."
                                        }
                                        _ => "No worlds found in the configured folder.",
                                    };
                                    ui.label(egui::RichText::new(text).color(theme::TEXT_DIM));
                                });
                            }
                            for i in 0..self.entries.len() {
                                let resp = Self::row(
                                    ui,
                                    &self.entries[i],
                                    self.selected == Some(i) && self.browsed.is_none(),
                                    64.0,
                                );
                                if resp.clicked() {
                                    self.selected = Some(i);
                                    self.browsed = None;
                                }
                                if resp.double_clicked() {
                                    result = Picked::Open(self.entries[i].path.clone());
                                }
                            }
                        });
                });
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("or").size(15.0));
                ui.add_space(8.0);
                // Java worlds are folders, Bedrock worlds folders or .mcworld files, and
                // console saves single files; each dialog only offers that kind.
                let mut picked = None;
                let label = self.edition.label();
                match self.edition {
                    UiEdition::Java => {
                        if widgets::button(
                            ui,
                            Some(Icon::Folder),
                            "Browse for a World...",
                            200.0,
                            true,
                        )
                        .clicked()
                        {
                            picked = settings.dialog("Select Java World Folder").pick_folder();
                        }
                    }
                    UiEdition::Bedrock => {
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 416.0) / 2.0);
                            if widgets::button(
                                ui,
                                Some(Icon::Folder),
                                "Browse for a Folder...",
                                200.0,
                                true,
                            )
                            .clicked()
                            {
                                picked =
                                    settings.dialog("Select Bedrock World Folder").pick_folder();
                            }
                            ui.add_space(8.0);
                            if widgets::button(
                                ui,
                                Some(Icon::File),
                                "Browse for a World File...",
                                200.0,
                                true,
                            )
                            .clicked()
                            {
                                picked = settings
                                    .dialog("Select Bedrock World File")
                                    .add_filter("Bedrock World (*.mcworld)", &["mcworld"])
                                    .pick_file();
                            }
                        });
                    }
                    console => {
                        if widgets::button(
                            ui,
                            Some(Icon::File),
                            "Browse for a World...",
                            200.0,
                            true,
                        )
                        .clicked()
                        {
                            let (name, exts): (&str, &[&str]) = match console {
                                UiEdition::Xbox360 => {
                                    ("Xbox 360 Save (*.dat, *.bin)", &["dat", "bin"])
                                }
                                // rfd 0.15.3 (pinned) writes each extension as "*.{ext}" into
                                // a ';'-separated Windows pattern, so this yields
                                // "*.GAMEDATA;GAMEDATA": the exact, extension-less file name.
                                UiEdition::Ps3 => {
                                    ("PlayStation 3 Save Data (GAMEDATA)", &["GAMEDATA;GAMEDATA"])
                                }
                                _ => ("Wii U Save (*.ext, *.wii)", &["ext", "wii"]),
                            };
                            picked = settings
                                .dialog(&format!("Select {label} World File"))
                                .add_filter(name, exts)
                                .pick_file();
                        }
                    }
                }
                {
                    if let Some(p) = picked {
                        settings.visited(&p);
                        match describe(&p, self.edition) {
                            Some(e) => {
                                self.browsed = Some(e);
                                self.error = None;
                            }
                            None => {
                                self.error =
                                    Some(format!("That is not a {} world.", self.edition.label()))
                            }
                        }
                    }
                }
                if let Some(e) = &self.error {
                    ui.add_space(4.0);
                    ui.colored_label(theme::RED, e);
                }
            });
            // Footer with the selection.
            let footer = Rect::from_min_max(
                pos2(ui.min_rect().left(), ui.max_rect().bottom() - 84.0),
                ui.max_rect().right_bottom(),
            );
            ui.painter().line_segment(
                [footer.left_top(), footer.right_top()],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            let chosen = self
                .browsed
                .as_ref()
                .or_else(|| self.selected.and_then(|i| self.entries.get(i)));
            if let Some(e) = chosen {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
                    footer.min + vec2(0.0, 6.0),
                    vec2(footer.width() - 180.0, 72.0),
                )));
                Self::row(&mut child, e, false, 72.0);
            }
            let br = Rect::from_min_size(
                pos2(footer.right() - 160.0, footer.top() + 16.0),
                vec2(148.0, 50.0),
            );
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
            if widgets::primary(&mut child, "Open World", br.size(), chosen.is_some()).clicked() {
                if let Some(e) = chosen {
                    result = Picked::Open(e.path.clone());
                }
            }
            ui.painter().rect_stroke(
                ui.max_rect(),
                theme::radius(6),
                Stroke::new(1.0_f32, theme::BORDER),
                StrokeKind::Inside,
            );
        });
        if response.should_close() && matches!(result, Picked::Nothing) {
            result = Picked::Cancelled;
        }
        result
    }
}
