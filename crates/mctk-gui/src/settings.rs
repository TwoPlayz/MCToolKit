//! Persisted settings (`%APPDATA%\MCToolKit\settings.json`) and the Settings page.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Ui};
use serde_json::{Value, json};

use crate::theme;
use crate::widgets;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// 0 none, 1 NBT Editor, 2 Converter (3, the Pruner, was folded into the NBT Editor).
    pub default_app: usize,
    /// The look: 0 as Windows is set (light or dark), 1 dark, 2 light.
    pub theme: usize,
    pub remember_position: bool,
    /// Outer window position and size.
    pub window: Option<[f32; 4]>,
    /// 0..=4 → 100 %, 125 %, 150 %, 175 %, 200 %.
    pub scale: usize,
    pub remember_dirs: bool,
    pub last_dir: Option<PathBuf>,
    pub java_worlds: String,
    pub bedrock_worlds: String,
    pub wiiu_worlds: String,
    pub xbox_worlds: String,
    pub ps3_worlds: String,
    pub working_dir: String,
    pub show_log: bool,
    /// Show only the messages that report a problem (the red ones).
    pub hide_messages: bool,
    pub log_right: bool,
    pub full_precision: bool,
    pub remember_conversion_options: bool,
    /// Last conversion options: overworld, nether, end, skip partial, game mode, difficulty.
    pub conversion: [i64; 6],
    /// Look for a newer release on GitHub when the app starts.
    pub check_updates: bool,
}

pub(crate) fn app_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MCToolKit")
}

/// `…\Minecraft Bedrock\Users\<id>\games\com.mojang\minecraftWorlds`, or the older UWP path.
fn default_bedrock_worlds() -> String {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    if let Some(users) = appdata
        .as_ref()
        .map(|a| a.join("Minecraft Bedrock").join("Users"))
    {
        if let Ok(entries) = std::fs::read_dir(&users) {
            for e in entries.flatten() {
                let p = e
                    .path()
                    .join("games")
                    .join("com.mojang")
                    .join("minecraftWorlds");
                if p.is_dir() {
                    return p.display().to_string();
                }
            }
        }
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|l| {
            PathBuf::from(l)
                .join(r"Packages\Microsoft.MinecraftUWP_8wekyb3d8bbwe\LocalState\games\com.mojang\minecraftWorlds")
                .display()
                .to_string()
        })
        .unwrap_or_default()
}

impl Default for Settings {
    fn default() -> Self {
        let java = std::env::var_os("APPDATA")
            .map(|a| {
                PathBuf::from(a)
                    .join(".minecraft")
                    .join("saves")
                    .display()
                    .to_string()
            })
            .unwrap_or_default();
        Settings {
            default_app: 0,
            theme: 0,
            remember_position: true,
            window: None,
            scale: 0,
            remember_dirs: true,
            last_dir: None,
            java_worlds: java,
            bedrock_worlds: default_bedrock_worlds(),
            wiiu_worlds: String::new(),
            xbox_worlds: String::new(),
            ps3_worlds: String::new(),
            working_dir: app_dir().display().to_string(),
            show_log: false,
            hide_messages: false,
            log_right: false,
            full_precision: false,
            remember_conversion_options: true,
            conversion: [1, 1, 1, 1, 0, 0],
            check_updates: true,
        }
    }
}

impl Settings {
    fn file() -> PathBuf {
        app_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let mut s = Settings::default();
        let Ok(text) = std::fs::read_to_string(Self::file()) else {
            return s;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return s;
        };
        let b = |k: &str, d: bool| v.get(k).and_then(Value::as_bool).unwrap_or(d);
        let st = |k: &str, d: &str| v.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        s.default_app = v
            .get("default_app")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            // (The Pruner, 3, is now part of the NBT Editor's World Explorer.)
            .min(3) as usize;
        if s.default_app == 3 {
            s.default_app = 1;
        }
        s.theme = v.get("theme").and_then(Value::as_u64).unwrap_or(0).min(2) as usize;
        s.remember_position = b("remember_position", s.remember_position);
        s.window = v.get("window").and_then(Value::as_array).and_then(|a| {
            let f: Vec<f32> = a
                .iter()
                .filter_map(|x| x.as_f64().map(|x| x as f32))
                .collect();
            (f.len() == 4).then(|| [f[0], f[1], f[2], f[3]])
        });
        s.scale = v.get("scale").and_then(Value::as_u64).unwrap_or(0).min(4) as usize;
        s.remember_dirs = b("remember_dirs", s.remember_dirs);
        s.last_dir = v.get("last_dir").and_then(Value::as_str).map(PathBuf::from);
        s.java_worlds = st("java_worlds", &s.java_worlds);
        s.bedrock_worlds = st("bedrock_worlds", &s.bedrock_worlds);
        s.wiiu_worlds = st("wiiu_worlds", &s.wiiu_worlds);
        s.xbox_worlds = st("xbox_worlds", &s.xbox_worlds);
        s.ps3_worlds = st("ps3_worlds", &s.ps3_worlds);
        s.working_dir = st("working_dir", &s.working_dir);
        s.check_updates = b("check_updates", s.check_updates);
        s.show_log = b("show_log", s.show_log);
        s.hide_messages = b("hide_messages", s.hide_messages);
        s.log_right = b("log_right", s.log_right);
        s.full_precision = b("full_precision", s.full_precision);
        s.remember_conversion_options =
            b("remember_conversion_options", s.remember_conversion_options);
        if let Some(a) = v.get("conversion").and_then(Value::as_array) {
            for (i, x) in a.iter().take(6).enumerate() {
                s.conversion[i] = x.as_i64().unwrap_or(s.conversion[i]);
            }
        }
        s
    }

    pub fn save(&self) {
        let v = json!({
            "default_app": self.default_app,
            "theme": self.theme,
            "remember_position": self.remember_position,
            "window": self.window,
            "scale": self.scale,
            "remember_dirs": self.remember_dirs,
            "last_dir": self.last_dir.as_ref().map(|p| p.display().to_string()),
            "java_worlds": self.java_worlds,
            "bedrock_worlds": self.bedrock_worlds,
            "wiiu_worlds": self.wiiu_worlds,
            "xbox_worlds": self.xbox_worlds,
            "ps3_worlds": self.ps3_worlds,
            "working_dir": self.working_dir,
            "check_updates": self.check_updates,
            "show_log": self.show_log,
            "hide_messages": self.hide_messages,
            "log_right": self.log_right,
            "full_precision": self.full_precision,
            "remember_conversion_options": self.remember_conversion_options,
            "conversion": self.conversion,
        });
        let path = Self::file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    pub fn zoom(&self) -> f32 {
        [1.0, 1.25, 1.5, 1.75, 2.0][self.scale.min(4)]
    }

    pub fn working_dir(&self) -> PathBuf {
        PathBuf::from(&self.working_dir)
    }

    /// A new, empty folder under `<working dir>/<app>/workspaces/`.
    pub fn new_workspace(&self, app: &str, name: &str) -> std::io::Result<PathBuf> {
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = self.working_dir().join(app).join("workspaces");
        for n in 1.. {
            let p = base.join(format!("{} {n}", safe.trim()));
            if !p.exists() {
                std::fs::create_dir_all(&p)?;
                return Ok(p);
            }
        }
        unreachable!()
    }

    /// A file dialog opened at the last visited directory (when remembered).
    pub fn dialog(&self, title: &str) -> rfd::FileDialog {
        let d = rfd::FileDialog::new().set_title(title);
        match (&self.last_dir, self.remember_dirs) {
            (Some(p), true) if p.is_dir() => d.set_directory(p),
            _ => d,
        }
    }

    pub fn visited(&mut self, path: &Path) {
        if self.remember_dirs {
            // Next time, open beside the chosen file or world folder.
            self.last_dir = Some(path.parent().unwrap_or(path).to_path_buf());
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(text)
            .font(theme::bold(26.0))
            .color(theme::strong()),
    );
    ui.add_space(14.0);
}

fn describe(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(RichText::new(text).size(13.5).color(theme::text_muted())).wrap());
    ui.add_space(20.0);
}

fn titled(ui: &mut Ui, title: &str, text: &str) {
    ui.label(
        RichText::new(title)
            .font(theme::bold(16.0))
            .color(theme::text()),
    );
    describe(ui, text);
}

fn folder_row(ui: &mut Ui, settings: &mut Settings, which: usize, title: &str, text: &str) -> bool {
    ui.label(RichText::new(title).font(theme::bold(16.0)));
    ui.add(egui::Label::new(
        RichText::new(text).size(13.5).color(theme::text_muted()),
    ));
    let mut changed = false;
    ui.horizontal(|ui| {
        let value = match which {
            0 => &mut settings.java_worlds,
            1 => &mut settings.bedrock_worlds,
            2 => &mut settings.wiiu_worlds,
            5 => &mut settings.xbox_worlds,
            6 => &mut settings.ps3_worlds,
            _ => &mut settings.working_dir,
        };
        changed |= widgets::text_field(ui, value, 510.0).changed();
        if widgets::button(ui, None, "Browse...", 80.0, true).clicked() {
            if let Some(p) = rfd::FileDialog::new().set_title(title).pick_folder() {
                *value = p.display().to_string();
                changed = true;
            }
        }
    });
    ui.add_space(20.0);
    changed
}

const LGPL_TEXT: &str = include_str!("../../../LICENSE");
const LATO_LICENSE: &str = include_str!("../assets/fonts/OFL.txt");
/// `name<TAB>version<TAB>license<TAB>repository`, one Rust dependency per line (generated
/// from `cargo metadata`; see `THIRD_PARTY_LICENSES.md`).
const RUST_DEPENDENCIES: &str = include_str!("../assets/rust-dependencies.tsv");

/// Non-Rust code, data and assets MCToolKit builds on: (name, what it is used for, license,
/// link, license text when it ships with the app).
const EXTERNAL: [(&str, &str, &str, &str, Option<&str>); 1] = [(
    "Lato font",
    "The app's typeface, by \u{141}ukasz Dziedzic",
    "SIL Open Font License 1.1",
    "https://www.latofonts.com",
    Some(LATO_LICENSE),
)];

fn license_text(ui: &mut Ui, id: &str, text: &str) {
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(240.0)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(text.trim())
                        .monospace()
                        .size(11.5)
                        .color(theme::text_muted()),
                )
                .wrap(),
            );
        });
    ui.add_space(6.0);
}

/// "About & Licenses": the MCToolKit license, then everything external it uses.
fn licenses(ui: &mut Ui) {
    ui.label(RichText::new("About & Licenses").font(theme::bold(16.0)));
    ui.add_space(8.0);
    ui.label(RichText::new(format!("MCToolKit {}", env!("CARGO_PKG_VERSION"))).color(theme::text()));
    ui.label(
        RichText::new("Minecraft is a trademark of Mojang Synergies AB; MCToolKit is not affiliated with Mojang or Microsoft.")
            .color(theme::text_dim()),
    );
    ui.add_space(14.0);

    ui.label(RichText::new("MCToolKit License").font(theme::bold(14.0)));
    ui.add(
        egui::Label::new(
            RichText::new("MCToolKit is free software under the GNU Lesser General Public License, version 3 or (at your option) any later version (LGPL-3.0-or-later). You can use, modify and share it; if you distribute a modified version of the MCToolKit code, its source must be available under the same license.")
                .size(13.5)
                .color(theme::text_muted()),
        )
        .wrap(),
    );
    egui::CollapsingHeader::new("Full license text")
        .id_salt("license-lgpl")
        .show(ui, |ui| license_text(ui, "license-lgpl-text", LGPL_TEXT));
    ui.add_space(14.0);

    ui.label(RichText::new("External Licenses").font(theme::bold(14.0)));
    ui.add(
        egui::Label::new(
            RichText::new("MCToolKit builds on the following work. Each keeps its own license, all compatible with the LGPL.")
                .size(13.5)
                .color(theme::text_muted()),
        )
        .wrap(),
    );
    ui.add_space(6.0);
    for (name, what, license, url, text) in EXTERNAL {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).font(theme::bold(13.5)).color(theme::text()));
            ui.label(RichText::new(format!("\u{2014} {license}")).size(13.5).color(theme::text_muted()));
            ui.hyperlink_to(RichText::new("link").size(13.0), url);
        });
        ui.add(egui::Label::new(RichText::new(what).size(13.0).color(theme::text_dim())).wrap());
        if let Some(text) = text {
            egui::CollapsingHeader::new("License text")
                .id_salt(("license", name))
                .show(ui, |ui| license_text(ui, name, text));
        }
        ui.add_space(6.0);
    }

    let deps: Vec<[&str; 4]> = RUST_DEPENDENCIES
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some([f.next()?, f.next()?, f.next()?, f.next().unwrap_or("")])
        })
        .collect();
    egui::CollapsingHeader::new(
        RichText::new(format!("Rust dependencies ({})", deps.len())).font(theme::bold(13.5)),
    )
    .id_salt("license-rust")
    .show(ui, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new("Crates compiled into MCToolKit, with the license each one is published under (where a crate offers a choice, MCToolKit uses the permissive option). The same list is in THIRD_PARTY_LICENSES.md.")
                    .size(13.0)
                    .color(theme::text_dim()),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("license-rust-list")
            .max_height(300.0)
            .show(ui, |ui| {
                for [name, version, license, repo] in &deps {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if repo.is_empty() {
                            ui.label(RichText::new(format!("{name} {version}")).size(12.5).color(theme::text()));
                        } else {
                            ui.hyperlink_to(RichText::new(format!("{name} {version}")).size(12.5), *repo);
                        }
                        ui.label(RichText::new(*license).size(12.5).color(theme::text_muted()));
                    });
                }
            });
    });
}

/// The Settings page. Returns true when something changed (and was saved).
pub fn page(ui: &mut Ui, settings: &mut Settings) -> bool {
    let before = settings.clone();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.add_space(6.0);
        egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 0)).show(ui, |ui| {
            ui.set_max_width(640.0);
            heading(ui, "General");
            titled(ui, "Default App", "Set the default app to open when the application starts.");
            ui.add_space(-12.0);
            widgets::segmented(ui, &mut settings.default_app, &["No Default", "NBT Editor", "Converter"], 0.0, true);
            ui.add_space(26.0);
            titled(ui, "Theme", "How the application looks: as Windows is set (light or dark), or always dark or light.");
            ui.add_space(-12.0);
            widgets::segmented(ui, &mut settings.theme, &["System", "Dark", "Light"], 0.0, true);
            ui.add_space(26.0);
            widgets::toggle_bold(ui, &mut settings.remember_position, "Remember Application Position");
            describe(ui, "Open the application at the same position that it was closed.");
            titled(ui, "Scale Factor", "Make the application appear larger. The NBT tree is not affected: it has its own scale in the tag bar.");
            ui.add_space(-12.0);
            widgets::segmented(ui, &mut settings.scale, &["100% (Normal)", "125%", "150%", "175%", "200%"], 0.0, true);
            ui.add_space(26.0);
            widgets::toggle_bold(ui, &mut settings.remember_dirs, "Remember Previously Visited Directories");
            describe(ui, "Open browse dialog windows at the previously visited directories.");
            folder_row(ui, settings, 0, "Java Worlds Folder", "The directory the world selection dialog uses to search for installed Java worlds.");
            folder_row(ui, settings, 1, "Bedrock Worlds Folder", "The directory the world selection dialog uses to search for installed Bedrock worlds.");
            folder_row(ui, settings, 5, "Xbox 360 Worlds Folder", "The directory (and its subfolders) the world selection dialog uses to search for Xbox 360 saves.");
            folder_row(ui, settings, 6, "PlayStation 3 Worlds Folder", "The directory (and its subfolders) the world selection dialog uses to search for PlayStation 3 saves.");
            folder_row(ui, settings, 2, "Wii U Worlds Folder", "The directory the world selection dialog uses to search for dumped Wii U worlds.");
            folder_row(ui, settings, 3, "Working Directory", "The location where temporary files are stored while working within the application.");
            let assets = match crate::assets::version() {
                Some(v) => format!("Blocks, items and mobs are drawn with Minecraft {v}'s own assets, fetched from mcasset.cloud as they are needed and kept. They are fetched again only when a newer Minecraft release comes out."),
                None => "Blocks, items and mobs are drawn with the latest Minecraft release's own assets, fetched from mcasset.cloud as they are needed and kept.".to_string(),
            };
            titled(ui, "Game Assets", &assets);
            ui.add_space(20.0);
            widgets::toggle_bold(ui, &mut settings.check_updates, "Check for Updates on Startup");
            describe(ui, "Ask GitHub for the newest MCToolKit release when the application starts and download and install it, then restart. If it cannot be installed here (the program is in a folder that cannot be written to), a notice in the title bar opens the download page instead.");
            widgets::toggle_bold(ui, &mut settings.hide_messages, "Hide Non-Warning Messages");
            describe(ui, "Only show the messages in the corner that report a problem. Messages that just say what was done (saved, opened, undone...) are not shown.");
            widgets::toggle_bold(ui, &mut settings.show_log, "Show Debug Log");
            describe(ui, "Displays useful information for finding bugs or developing scripts.");
            titled(ui, "Debug Log Position", "Set where the debug log should be positioned.");
            ui.add_space(-12.0);
            let mut pos = settings.log_right as usize;
            if widgets::segmented(ui, &mut pos, &["Bottom", "Right"], 0.0, true) {
                settings.log_right = pos == 1;
            }
            ui.add_space(26.0);
            ui.separator();

            heading(ui, "NBT Editor");
            describe(ui, "Nothing is changed on disk until you save: edits are held in memory, so worlds are not copied when opened. When you save, every file is backed up as <name>_MCTBackup before it is first overwritten.");
            widgets::toggle_bold(ui, &mut settings.full_precision, "Full Floating Point Precision");
            describe(ui, "Display full floating-point precision for Float and Double NBT tags.");
            ui.separator();

            heading(ui, "Converter");
            widgets::toggle_bold(ui, &mut settings.remember_conversion_options, "Remember Conversion Options");
            describe(ui, "Remembers the conversion options for the next time a world is converted. The converter never modifies the input world, so it is not copied first.");
            ui.separator();

            heading(ui, "Community");
            describe(ui, "Ask questions, share worlds and report problems with conversions or the tools on the MCToolKit Discord server.");
            if widgets::button(ui, None, "Join the community", 0.0, true).clicked() {
                widgets::open_discord(ui);
            }
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(20.0);
            licenses(ui);
            ui.add_space(30.0);
        });
    });
    let changed = *settings != before;
    if changed {
        settings.save();
    }
    changed
}
