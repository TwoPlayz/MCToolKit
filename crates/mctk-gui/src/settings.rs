//! Persisted settings (`%APPDATA%\MCToolKit\settings.json`) and the Settings page.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Ui};
use serde_json::{Value, json};

use crate::theme;
use crate::widgets;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// 0 none, 1 NBT Editor, 2 Converter, 3 Pruner.
    pub default_app: usize,
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
    pub log_right: bool,
    pub editor_copy_bedrock: bool,
    pub editor_copy_java: bool,
    pub full_precision: bool,
    pub remember_conversion_options: bool,
    /// Last conversion options: overworld, nether, end, skip partial, game mode, difficulty.
    pub conversion: [i64; 6],
    pub disabled_extensions: Vec<String>,
    /// A Minecraft resource pack folder whose textures draw block and item icons.
    pub resource_pack: String,
}

fn app_dir() -> PathBuf {
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
            log_right: false,
            editor_copy_bedrock: true,
            editor_copy_java: false,
            full_precision: false,
            remember_conversion_options: true,
            conversion: [1, 1, 1, 1, 0, 0],
            disabled_extensions: Vec::new(),
            resource_pack: crate::resource_pack::default_dir(&app_dir())
                .display()
                .to_string(),
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
            .min(3) as usize;
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
        s.resource_pack = st("resource_pack", &s.resource_pack);
        if s.resource_pack.is_empty() {
            s.resource_pack = crate::resource_pack::default_dir(&s.working_dir())
                .display()
                .to_string();
        }
        s.show_log = b("show_log", s.show_log);
        s.log_right = b("log_right", s.log_right);
        s.editor_copy_bedrock = b("editor_copy_bedrock", s.editor_copy_bedrock);
        s.editor_copy_java = b("editor_copy_java", s.editor_copy_java);
        s.full_precision = b("full_precision", s.full_precision);
        s.remember_conversion_options =
            b("remember_conversion_options", s.remember_conversion_options);
        if let Some(a) = v.get("conversion").and_then(Value::as_array) {
            for (i, x) in a.iter().take(6).enumerate() {
                s.conversion[i] = x.as_i64().unwrap_or(s.conversion[i]);
            }
        }
        s.disabled_extensions = v
            .get("disabled_extensions")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        s
    }

    pub fn save(&self) {
        let v = json!({
            "default_app": self.default_app,
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
            "resource_pack": self.resource_pack,
            "show_log": self.show_log,
            "log_right": self.log_right,
            "editor_copy_bedrock": self.editor_copy_bedrock,
            "editor_copy_java": self.editor_copy_java,
            "full_precision": self.full_precision,
            "remember_conversion_options": self.remember_conversion_options,
            "conversion": self.conversion,
            "disabled_extensions": self.disabled_extensions,
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

    pub fn extensions_dir(&self) -> PathBuf {
        self.working_dir().join("extensions")
    }

    /// `<resource pack>/assets/minecraft/textures`, when the pack is set and present.
    pub fn texture_pack(&self) -> Option<PathBuf> {
        let p = PathBuf::from(&self.resource_pack).join(r"assets\minecraft\textures");
        (!self.resource_pack.is_empty() && p.is_dir()).then_some(p)
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
            .color(egui::Color32::WHITE),
    );
    ui.add_space(14.0);
}

fn describe(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.add(egui::Label::new(RichText::new(text).size(13.5).color(theme::TEXT_MUTED)).wrap());
    ui.add_space(20.0);
}

fn titled(ui: &mut Ui, title: &str, text: &str) {
    ui.label(
        RichText::new(title)
            .font(theme::bold(16.0))
            .color(theme::TEXT),
    );
    describe(ui, text);
}

fn folder_row(ui: &mut Ui, settings: &mut Settings, which: usize, title: &str, text: &str) -> bool {
    ui.label(RichText::new(title).font(theme::bold(16.0)));
    ui.add(egui::Label::new(
        RichText::new(text).size(13.5).color(theme::TEXT_MUTED),
    ));
    let mut changed = false;
    ui.horizontal(|ui| {
        let value = match which {
            0 => &mut settings.java_worlds,
            1 => &mut settings.bedrock_worlds,
            2 => &mut settings.wiiu_worlds,
            4 => &mut settings.resource_pack,
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
            widgets::segmented(ui, &mut settings.default_app, &["No Default", "NBT Editor", "Converter", "Pruner"], 0.0, true);
            ui.add_space(26.0);
            widgets::toggle_bold(ui, &mut settings.remember_position, "Remember Application Position");
            describe(ui, "Open the application at the same position that it was closed.");
            titled(ui, "Scale Factor", "Make the application appear larger.");
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
            folder_row(ui, settings, 3, "Working Directory", "The location where temporary files and extensions are stored while working within the application.");
            folder_row(ui, settings, 4, "Resource Pack", "A Minecraft resource pack folder whose block and item textures are used for icons and map colors. Defaults to MCToolKit's own pack, filled on first run from Universal Minecraft Tool or Minecraft Java when either is installed.");
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
            widgets::toggle_bold(ui, &mut settings.editor_copy_bedrock, "Copy Bedrock Worlds (Recommended)");
            describe(ui, "Opened Bedrock worlds are copied before work begins, and saving writes the copy back (keeping a _MCTBackup.zip of the original). Disable only if the world you're working on is too large to be copied and you know what you're doing. (All Bedrock worlds in .mcworld format are copied regardless.)");
            widgets::toggle_bold(ui, &mut settings.editor_copy_java, "Copy Java Worlds (Not Recommended)");
            describe(ui, "Opened Java worlds are copied before work begins. Only use this for safety reasons, but is mostly unnecessary: without it, every file is backed up as <name>_MCTBackup before it is first overwritten.");
            widgets::toggle_bold(ui, &mut settings.full_precision, "Full Floating Point Precision");
            describe(ui, "Display full floating-point precision for Float and Double NBT tags.");
            ui.separator();

            heading(ui, "Converter");
            widgets::toggle_bold(ui, &mut settings.remember_conversion_options, "Remember Conversion Options");
            describe(ui, "Remembers the conversion options for the next time a world is converted. The converter never modifies the input world, so it is not copied first.");
            ui.separator();

            heading(ui, "Pruner");
            describe(ui, "The pruner always works on a copy of the opened world: pruned chunks are removed from the copy, and Save writes the copy back over the original after keeping a _MCTBackup of it.");
            ui.separator();

            heading(ui, "Community");
            describe(ui, "Ask questions, share worlds and report problems with conversions or the tools on the MCToolKit Discord server.");
            if widgets::button(ui, None, "Join the community", 0.0, true).clicked() {
                widgets::open_discord(ui);
            }
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("About & Licenses").font(theme::bold(16.0)));
                ui.add_space(8.0);
                ui.label(RichText::new(format!("MCToolKit {} — LGPL-3.0-or-later", env!("CARGO_PKG_VERSION"))).color(theme::TEXT_MUTED));
                ui.label(RichText::new("Block and biome mappings: GeyserMC (MIT). Font: Lato (SIL Open Font License 1.1).").color(theme::TEXT_MUTED));
                ui.label(RichText::new("Minecraft is a trademark of Mojang Synergies AB; MCToolKit is not affiliated with Mojang or Microsoft.").color(theme::TEXT_DIM));
            });
            ui.add_space(30.0);
        });
    });
    let changed = *settings != before;
    if changed {
        settings.save();
    }
    changed
}
