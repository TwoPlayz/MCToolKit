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

    /// What to call a world in a message: its name for Java and Bedrock, which have world
    /// names, and "This <edition> world" for the consoles, whose saves are all called alike
    /// (GAMEDATA, savegame.dat...).
    pub fn subject(self, name: &str) -> String {
        match self {
            UiEdition::Java | UiEdition::Bedrock => name.to_string(),
            other => format!("This {} world", other.label()),
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
    /// The folder a Java or Bedrock world is in, shown after its name when that differs.
    folder: String,
    /// Game mode and difficulty, as the game lists them ("Creative, Normal").
    detail: String,
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

/// "Creative, Normal": how the game lists a world's mode and difficulty.
fn world_detail(mode: Option<i64>, difficulty: Option<i64>, hardcore: bool) -> String {
    let mode = match mode {
        Some(0) => Some("Survival"),
        Some(1) => Some("Creative"),
        Some(2) => Some("Adventure"),
        Some(3) => Some("Spectator"),
        _ => None,
    };
    let difficulty = match difficulty {
        Some(0) => Some("Peaceful"),
        Some(1) => Some("Easy"),
        Some(2) => Some("Normal"),
        Some(3) => Some("Hard"),
        _ => None,
    };
    let mode = if hardcore { Some("Hardcore") } else { mode };
    [mode, difficulty].into_iter().flatten().collect::<Vec<_>>().join(", ")
}

/// A world's picture, or Minecraft's pack.png when it has none.
fn draw_picture(p: &egui::Painter, rect: Rect, e: &Entry) {
    if !e.icon.as_deref().is_some_and(|i| icons::file_image(p, rect, i))
        && !icons::pack_file("pack.png").is_some_and(|i| icons::file_image(p, rect, &i))
    {
        p.rect_filled(rect, theme::radius(3), theme::chip());
    }
}

/// A path with its middle left out when it is long.
fn short_path(path: &Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    if path.chars().count() > 40 {
        let tail: String = path.chars().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{}...{}", path.chars().take(17).collect::<String>(), tail)
    } else {
        path
    }
}

/// Opens the file explorer on the world: its folder, or the folder holding its file.
fn show_in_explorer(path: &Path) {
    let mut command = std::process::Command::new("explorer");
    if path.is_file() {
        command.arg(format!("/select,{}", path.display()));
    } else {
        command.arg(path);
    }
    let _ = command.spawn();
}

/// The days since 1970-01-01 as (year, month, day).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// `utc` seconds since 1970 as seconds in the user's time zone (as it was on that date).
#[cfg(windows)]
fn to_local(utc: i64) -> i64 {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SystemTimeToTzSpecificLocalTime(zone: *const u8, utc: *const SystemTime, local: *mut SystemTime) -> i32;
    }
    let (year, month, day) = civil_date(utc.div_euclid(86_400));
    let rest = utc.rem_euclid(86_400);
    let from = SystemTime {
        year: year as u16,
        month: month as u16,
        day: day as u16,
        hour: (rest / 3600) as u16,
        minute: (rest % 3600 / 60) as u16,
        second: (rest % 60) as u16,
        ..Default::default()
    };
    let mut to = SystemTime::default();
    // SAFETY: both pointers are to live structs of the layout Windows documents; a null zone
    // means the current one.
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &from, &mut to) } == 0 {
        return utc;
    }
    // Back to seconds since 1970 (the same days arithmetic, the other way).
    let (y, m) = (i64::from(to.year) - i64::from(to.month <= 2), i64::from(to.month));
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(to.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + i64::from(to.hour) * 3600 + i64::from(to.minute) * 60 + i64::from(to.second)
}

#[cfg(not(windows))]
fn to_local(utc: i64) -> i64 {
    utc
}

/// "11:33pm, Oct 28, 2026": when a world was last played, in the user's time zone.
fn last_played_text(utc: i64) -> String {
    if utc <= 0 {
        return String::new();
    }
    let local = to_local(utc);
    let (year, month, day) = civil_date(local.div_euclid(86_400));
    let rest = local.rem_euclid(86_400);
    let (hour, minute) = (rest / 3600, rest % 3600 / 60);
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}:{minute:02}{}, {} {day}, {year}",
        if hour % 12 == 0 { 12 } else { hour % 12 },
        if hour < 12 { "am" } else { "pm" },
        months[month as usize - 1]
    )
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
            let mode = data.and_then(|d| d.get("GameType")).and_then(mctk_nbt::Tag::as_i64);
            // 26.x keeps the difficulty in difficulty_settings as a word; before it was a byte.
            let difficulty = data
                .and_then(|d| d.get("Difficulty").and_then(mctk_nbt::Tag::as_i64))
                .or_else(|| {
                    let word = data?.get("difficulty_settings")?.get("difficulty")?.as_str()?;
                    ["peaceful", "easy", "normal", "hard"].iter().position(|d| *d == word).map(|i| i as i64)
                });
            let hardcore = data
                .and_then(|d| d.get("hardcore").or_else(|| d.get("difficulty_settings")?.get("hardcore")))
                .and_then(mctk_nbt::Tag::as_i64)
                == Some(1);
            Some(Entry {
                folder: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                detail: world_detail(mode, difficulty, hardcore),
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
            let level = std::fs::read(path.join("level.dat")).ok().and_then(|b| mctk_bedrock::LevelDat::parse(&b).ok());
            let number = |key: &str| level.as_ref().and_then(|l| l.root.tag.get(key)).and_then(mctk_nbt::Tag::as_i64);
            Some(Entry {
                folder: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                detail: world_detail(number("GameType"), number("Difficulty"), false),
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
            // The save keeps its own level.dat: the game mode and difficulty are in it.
            let data = mctk_console::ConsoleSave::open(path).ok().and_then(|save| {
                let entry = save.archive.get("level.dat")?;
                let (_, raw) = mctk_nbt::decompress(&entry.data).ok()?;
                let root = mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java).ok()?;
                root.tag.get("Data").cloned()
            });
            let number = |key: &str| data.as_ref().and_then(|d| d.get(key)).and_then(mctk_nbt::Tag::as_i64);
            let detail = world_detail(number("GameType"), number("Difficulty"), number("hardcore") == Some(1));
            let played = number("LastPlayed").filter(|ms| *ms > 0).map(|ms| ms / 1000);
            Some(Entry {
                name: shown
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: path.to_path_buf(),
                last_played: played.unwrap_or_else(|| mtime(path)),
                icon: folder.map(|f| f.join("ICON0.PNG")),
                folder: String::new(),
                detail,
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
            p.rect_filled(rect, 0.0, theme::button());
        }
        let icon = Rect::from_min_size(rect.min + vec2(8.0, 6.0), Vec2::splat(height - 12.0));
        if !e
            .icon
            .as_deref()
            .is_some_and(|i| icons::file_image(p, icon, i))
            // Minecraft's pack.png for worlds without an image.
            && !icons::pack_file("pack.png").is_some_and(|i| icons::file_image(p, icon, &i))
        {
            p.rect_filled(icon, theme::radius(3), theme::chip());
        }
        let x = icon.right() + 14.0;
        let name = p.layout_no_wrap(e.name.clone(), theme::bold(15.0), theme::text());
        let name_width = name.size().x;
        p.galley(pos2(x, rect.center().y - 9.0 - name.size().y / 2.0), name, theme::text());
        // The folder, after the name, when it is not the name itself (Bedrock folders never are).
        if !e.folder.is_empty() && e.folder != e.name {
            p.text(
                pos2(x + name_width + 6.0, rect.center().y - 9.0),
                Align2::LEFT_CENTER,
                format!("- {}", e.folder),
                theme::regular(14.0),
                theme::text_dim(),
            );
        }
        let when = last_played_text(e.last_played);
        if !when.is_empty() {
            p.text(
                pos2(rect.right() - 12.0, rect.center().y + 11.0),
                Align2::RIGHT_CENTER,
                when,
                theme::regular(12.5),
                theme::text_muted(),
            );
        }
        // Java and Bedrock worlds show their game mode and difficulty; a console save has only a
        // path to show.
        if !e.detail.is_empty() {
            p.text(
                pos2(x, rect.center().y + 11.0),
                Align2::LEFT_CENTER,
                &e.detail,
                theme::regular(14.0),
                theme::text_muted(),
            );
            return resp;
        }
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
            theme::text_muted(),
        );
        resp
    }

    /// The chosen world under the list: its picture, name and path, as one card that lights up
    /// under the pointer. Clicking it shows the world in the file explorer.
    fn footer(ui: &mut egui::Ui, e: &Entry, height: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Show in the file explorer");
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, 0.0, theme::button());
        }
        let icon = Rect::from_min_size(rect.min + vec2(8.0, 6.0), Vec2::splat(height - 12.0));
        draw_picture(p, icon, e);
        let x = icon.right() + 14.0;
        p.text(
            pos2(x, rect.center().y - 9.0),
            Align2::LEFT_CENTER,
            &e.name,
            theme::bold(15.0),
            theme::text(),
        );
        p.text(
            pos2(x, rect.center().y + 11.0),
            Align2::LEFT_CENTER,
            short_path(&e.path),
            theme::regular(12.5),
            theme::text_muted(),
        );
        if resp.clicked() {
            show_in_explorer(&e.path);
        }
    }
    /// Draws the dialog; returns what the user did this frame.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Picked {
        let mut result = Picked::Nothing;
        let size = vec2(520.0, 650.0f32.min(ctx.screen_rect().height() - 40.0));
        let modal = egui::Modal::new(egui::Id::new("world-picker")).frame(
            egui::Frame::new()
                .fill(theme::card())
                .stroke(Stroke::new(1.0_f32, theme::border()))
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
                theme::text(),
            );
            // The small X of the tabs and the messages: it lights up under the pointer.
            let close = Rect::from_center_size(bar.right_center() - vec2(22.0, 0.0), Vec2::splat(28.0));
            let cr = crate::editor::close_button(ui, close, egui::Id::new("picker-close"));
            if cr.clicked() {
                result = Picked::Cancelled;
            }
            // Sort / folder / refresh row.
            ui.horizontal(|ui| {
                ui.add_space(30.0);
                ui.label(egui::RichText::new("Sort by:").color(theme::text()));
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
                .fill(theme::bg())
                .stroke(Stroke::new(1.0_f32, theme::border()))
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
                                    ui.label(egui::RichText::new(text).color(theme::text_dim()));
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
                Stroke::new(1.0_f32, theme::border()),
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
                Self::footer(&mut child, e, 72.0);
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
                Stroke::new(1.0_f32, theme::border()),
                StrokeKind::Inside,
            );
        });
        if response.should_close() && matches!(result, Picked::Nothing) {
            result = Picked::Cancelled;
        }
        result
    }
}
