//! The notes the NBT editor adds beside tags: what a game mode number means, how long a tick
//! count is, what a position or rotation points at, which version a `DataVersion` is, and a
//! picture for the player. They are worked out from the tags themselves and shown in grey after
//! the value.

use std::collections::HashMap;

use mctk_nbt::{Compound, Tag};

use crate::icons::Icon;
use crate::nbt_view::{Seg, TagPath};

/// A note beside one tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagStyle {
    pub label: Option<String>,
    /// A colour name ("lime", "gold", ...) or `#rrggbb`.
    pub color: Option<String>,
    /// A small picture drawn before the label.
    pub icon: Option<NoteIcon>,
}

/// Where a note's picture comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoteIcon {
    /// The first of these textures the resource pack has (paths under `assets/minecraft/textures`).
    Pack(&'static [&'static str]),
    /// One of the app's own icons.
    Own(Icon),
    /// One of the bundled vector icons by name (`POS`, `ROT`).
    Image(&'static str),
}

const HEART: NoteIcon = NoteIcon::Pack(&["gui/sprites/hud/heart/full.png", "item/apple.png"]);
const GOLD_HEART: NoteIcon = NoteIcon::Pack(&["gui/sprites/hud/heart/absorbing_full.png", "item/golden_apple.png"]);
const HUNGER: NoteIcon = NoteIcon::Pack(&["gui/sprites/hud/food_full.png", "item/cooked_beef.png"]);
const FIRE: NoteIcon = NoteIcon::Pack(&["block/fire_0.png", "item/fire_charge.png", "item/flint_and_steel.png"]);

pub type Styles = HashMap<TagPath, TagStyle>;

/// What kind of file a document is.
pub mod file_type {
    pub const LEVEL: u32 = 1;
    pub const PLAYER: u32 = 2;
    pub const CHUNK: u32 = 4;
    pub const MAP: u32 = 8;
    pub const OTHER: u32 = 16;
}

/// Which edition's layout a document has.
pub mod edition {
    pub const JAVA: i64 = 0;
    pub const BEDROCK: i64 = 1;
    pub const CONSOLE: i64 = 2;
}

const GREY: &str = "#bfbfbf";
const DIM: &str = "#56616f";

/// The notes for a whole document.
pub fn style_tree(root: &Tag, edition: i64, file: u32) -> Styles {
    let mut s = Styler { out: Styles::new(), edition };
    let mut path = TagPath::new();
    s.data_versions(root, &mut path);
    if file & file_type::LEVEL != 0 {
        s.world_settings(root);
    }
    if file & (file_type::PLAYER | file_type::LEVEL) != 0 {
        s.player(root, file);
    }
    s.out
}

struct Styler {
    out: Styles,
    edition: i64,
}

fn key(p: &TagPath, k: &str) -> TagPath {
    let mut c = p.clone();
    c.push(Seg::Key(k.to_string()));
    c
}

fn index(p: &TagPath, i: usize) -> TagPath {
    let mut c = p.clone();
    c.push(Seg::Index(i));
    c
}

fn compound(t: &Tag) -> Option<&Compound> {
    match t {
        Tag::Compound(c) => Some(c),
        _ => None,
    }
}

impl Styler {
    fn put(&mut self, p: TagPath, label: impl Into<String>, color: Option<&str>) {
        let e = self.out.entry(p).or_default();
        e.label = Some(label.into());
        if let Some(c) = color {
            e.color = Some(c.to_string());
        }
    }

    fn icon(&mut self, p: TagPath, icon: NoteIcon) {
        self.out.entry(p).or_default().icon = Some(icon);
    }

    /// A `DataVersion` says which game version wrote the file.
    fn data_versions(&mut self, t: &Tag, path: &mut TagPath) {
        match t {
            Tag::Compound(c) => {
                for (k, v) in c {
                    path.push(Seg::Key(k.clone()));
                    if let (true, Tag::Int(n)) = (k == "DataVersion", v) {
                        if let Some(name) = data_version_name(*n) {
                            self.put(path.clone(), name, Some(DIM));
                        }
                    }
                    self.data_versions(v, path);
                    path.pop();
                }
            }
            Tag::List(l) => {
                for (i, v) in l.iter().enumerate() {
                    path.push(Seg::Index(i));
                    self.data_versions(v, path);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ the world's settings

    fn world_settings(&mut self, root: &Tag) {
        let Some(top) = compound(root) else { return };
        let (c, at) = match (self.edition, top.get("Data")) {
            (edition::BEDROCK, _) => (top, TagPath::new()),
            (_, Some(Tag::Compound(d))) => (d, key(&TagPath::new(), "Data")),
            _ => return,
        };
        let bedrock = self.edition == edition::BEDROCK;
        if let Some(Tag::Int(v)) = c.get("GameType") {
            let name = match v {
                0 => Some(("Survival", "lime")),
                1 => Some(("Creative", "gold")),
                2 => Some(("Adventure", "cyan")),
                3 if self.edition == edition::JAVA => Some(("Spectator", "white")),
                _ => None,
            };
            if let Some((n, col)) = name {
                self.put(key(&at, "GameType"), n, Some(col));
            }
        }
        let difficulty = match c.get("Difficulty") {
            Some(Tag::Byte(v)) => Some(*v as i64),
            Some(Tag::Int(v)) if bedrock => Some(*v as i64),
            _ => None,
        };
        if let Some(v) = difficulty {
            let name = match v {
                0 => Some(("Peaceful", "cyan")),
                1 => Some(("Easy", "lime")),
                2 => Some(("Normal", "gold")),
                3 => Some(("Hard", "red")),
                _ => None,
            };
            if let Some((n, col)) = name {
                self.put(key(&at, "Difficulty"), n, Some(col));
            }
        }
        if let Some(Tag::Long(v)) = c.get("LastPlayed") {
            // Java stores milliseconds, Bedrock seconds.
            let secs = if bedrock { *v } else { v.div_euclid(1000) };
            self.put(key(&at, "LastPlayed"), format_time(secs), Some(DIM));
        }
        if bedrock {
            if let Some(Tag::Long(v)) = c.get("currentTick") {
                self.put(key(&at, "currentTick"), long_time(*v / 20), Some(DIM));
            }
            if let Some(Tag::Int(v)) = c.get("rainTime") {
                self.put(key(&at, "rainTime"), format!("Rain toggles in {}", long_time(*v as i64 / 20)), Some(DIM));
            }
            if let Some(Tag::Int(v)) = c.get("Generator") {
                let name = match v {
                    0 => Some("Old"),
                    1 => Some("Infinite"),
                    2 => Some("Flat"),
                    _ => None,
                };
                if let Some(n) = name {
                    self.put(key(&at, "Generator"), n, None);
                }
            }
            return;
        }
        if let Some(Tag::Long(v)) = c.get("Time") {
            if *v >= 0 {
                self.put(key(&at, "Time"), long_time(*v / 20), Some(DIM));
            }
        }
        if let Some(Tag::Long(v)) = c.get("DayTime") {
            if *v >= 0 {
                self.put(key(&at, "DayTime"), day_time(*v), Some(DIM));
            }
        }
        for (time, flag, stops, starts) in [
            ("rainTime", "raining", "Rain stops in ", "Rain starts in "),
            ("thunderTime", "thundering", "Thunder stops in ", "Thunder starts in "),
        ] {
            if let Some(Tag::Int(v)) = c.get(time) {
                if *v >= 20 {
                    let lead = match c.get(flag) {
                        Some(Tag::Byte(0)) => starts,
                        _ => stops,
                    };
                    self.put(key(&at, time), format!("{lead}{}", long_time(*v as i64 / 20)), Some(DIM));
                }
            }
        }
        if self.edition == edition::CONSOLE {
            if let Some(Tag::Long(v)) = c.get("SizeOnDisk") {
                let mb = (*v as f64 / 1_000_000.0 * 100.0 + 0.5).floor() / 100.0;
                self.put(key(&at, "SizeOnDisk"), format!("{mb}MB"), Some(DIM));
            }
            if let Some(Tag::Int(v)) = c.get("generatorVersion") {
                match v {
                    0 => self.put(key(&at, "generatorVersion"), "Flat", Some(DIM)),
                    1 => self.put(key(&at, "generatorVersion"), "Normal", Some(DIM)),
                    _ => {}
                }
            }
        }
    }

    // ------------------------------------------------------------------------- the player

    fn player(&mut self, root: &Tag, file: u32) {
        let Some(top) = compound(root) else { return };
        let mut at = TagPath::new();
        let mut player = top;
        // level.dat keeps the player inside Data.
        if let Some(Tag::Compound(data)) = top.get("Data") {
            at = key(&at, "Data");
            player = data;
            if let Some(Tag::Compound(p)) = data.get("Player") {
                at = key(&at, "Player");
                player = p;
            } else if file & file_type::PLAYER == 0 {
                // A level without a player has nothing to say here.
                return;
            }
        } else if file & file_type::PLAYER == 0 {
            return;
        }
        match self.edition {
            edition::JAVA => self.java_player(player, &at),
            edition::BEDROCK => self.bedrock_player(player, &at),
            _ => self.console_player(player, &at),
        }
        self.entity(player, &at);
    }

    fn game_mode(&mut self, c: &Compound, at: &TagPath, name: &str) {
        if let Some(Tag::Int(v)) = c.get(name) {
            let java = self.edition != edition::BEDROCK;
            let mode = match (java, v) {
                (_, 0) => Some(("Survival", "lime")),
                (_, 1) => Some(("Creative", "gold")),
                (_, 2) => Some(("Adventure", "cyan")),
                (true, 3) => Some(("Spectator", "white")),
                (false, 5) => Some(("Using world default", "white")),
                (false, 6) => Some(("Spectator", "white")),
                _ => None,
            };
            if let Some((n, col)) = mode {
                self.put(key(at, name), n, Some(col));
            }
        }
    }

    /// Notes the three editions share.
    fn common_player(&mut self, c: &Compound, at: &TagPath) {
        if let Some(Tag::Short(v)) = c.get("SleepTimer") {
            let text = match v {
                0 => "Not in bed",
                1..=100 => "In bed",
                101..=109 => "Just woke up",
                _ => "",
            };
            if !text.is_empty() {
                self.put(key(at, "SleepTimer"), text, None);
            }
        }
        if let Some(Tag::Float(v)) = c.get("FallDistance") {
            let mut text = if *v == 0.0 {
                "Blocks fallen".to_string()
            } else if *v > 0.0 {
                format!("{} blocks fallen", v.floor() as i64)
            } else {
                String::new()
            };
            if *v > 3.0 {
                text += &format!(" ({} hearts of damage)", trim_zero((*v - 3.0) / 2.0));
            }
            self.put(key(at, "FallDistance"), text, None);
        }
    }

    fn xp(&mut self, c: &Compound, at: &TagPath, level_note: &str) {
        if c.get("XpLevel").is_some_and(|t| matches!(t, Tag::Int(_))) {
            self.put(key(at, "XpLevel"), level_note, None);
        }
        if let Some(Tag::Float(v)) = c.get("XpP") {
            self.put(key(at, "XpP"), format!("{}% to next XP level", (v * 100.0).floor() as i64), None);
        }
        if c.get("XpTotal").is_some_and(|t| matches!(t, Tag::Int(_))) {
            self.put(key(at, "XpTotal"), "Total lifetime XP. Usually the same as Score", None);
        }
        if c.get("Score").is_some_and(|t| matches!(t, Tag::Int(_))) {
            self.put(key(at, "Score"), "Score shown on death", None);
        }
    }

    fn food(&mut self, c: &Compound, at: &TagPath) {
        if let Some(Tag::Int(v)) = c.get("foodLevel") {
            let mut text = format!("{} Food", trim_zero(*v as f32 / 2.0));
            match v {
                0 => text += " (Starving)",
                10 => text += " (Half)",
                20 => text += " (Full)",
                _ => {}
            }
            self.put(key(at, "foodLevel"), text, None);
            self.icon(key(at, "foodLevel"), HUNGER);
        }
    }

    fn java_player(&mut self, c: &Compound, at: &TagPath) {
        self.game_mode(c, at, "playerGameType");
        self.game_mode(c, at, "previousPlayerGameType");
        self.xp(c, at, "Level shown on the XP bar");
        self.food(c, at);
        if let Some(Tag::Compound(death)) = c.get("LastDeathLocation") {
            if let Some(Tag::IntArray(pos)) = death.get("pos") {
                if pos.len() == 3 {
                    let mut text = format!("X:{}, Y:{}, Z:{}", pos[0], pos[1], pos[2]);
                    if let Some(Tag::String(dim)) = death.get("dimension") {
                        match dim.as_str() {
                            "minecraft:overworld" => text += " in the Overworld",
                            "minecraft:nether" => text += " in the Nether",
                            "minecraft:end" => text += " in the End",
                            _ => {}
                        }
                    }
                    self.put(key(at, "LastDeathLocation"), text, Some(GREY));
                }
            }
        }
        self.common_player(c, at);
    }

    fn bedrock_player(&mut self, c: &Compound, at: &TagPath) {
        self.game_mode(c, at, "PlayerGameMode");
        self.common_player(c, at);
        let dimension = |v: i32| match v {
            0 => Some("Overworld"),
            1 => Some("Nether"),
            2 => Some("The End"),
            _ => None,
        };
        if let Some(Tag::Int(v)) = c.get("DeathDimension") {
            if let Some(n) = dimension(*v) {
                self.put(key(at, "DeathDimension"), n, None);
            }
        }
        if let Some(Tag::Int(v)) = c.get("DimensionId") {
            let named = match v {
                0 => Some(("Overworld", "lime")),
                1 => Some(("Nether", "red")),
                2 => Some(("The End", "magenta")),
                _ => None,
            };
            if let Some((n, col)) = named {
                self.put(key(at, "DimensionId"), n, Some(col));
            }
        }
        if let Some(Tag::Int(v)) = c.get("SpawnDimension") {
            // 3 probably means the world default, shown as the Overworld.
            let n = if *v == 3 { Some("Overworld") } else { dimension(*v) };
            if let Some(n) = n {
                self.put(key(at, "SpawnDimension"), n, None);
            }
        }
        if c.get("PlayerLevel").is_some_and(|t| matches!(t, Tag::Int(_))) {
            self.put(key(at, "PlayerLevel"), "Level shown on the XP bar", None);
        }
        if let Some(Tag::Float(v)) = c.get("PlayerLevelProgress") {
            self.put(
                key(at, "PlayerLevelProgress"),
                format!("{}% to next XP level", (v * 100.0).floor() as i64),
                None,
            );
        }
        if let Some(Tag::Int(v)) = c.get("TimeSinceRest") {
            self.put(key(at, "TimeSinceRest"), ticks_to_time(*v as f64, false), None);
        }
        if let Some(Tag::List(list)) = c.get("Attributes") {
            for (i, a) in list.iter().enumerate() {
                if let Some(Tag::String(name)) = compound(a).and_then(|a| a.get("Name")) {
                    let text = match name.as_str() {
                        "minecraft:player.hunger" => "Hunger",
                        "minecraft:player.exhaustion" => "Exhaustion",
                        "minecraft:player.saturation" => "Saturation",
                        "minecraft:player.level" => "Level",
                        "minecraft:player.experience" => "Experience",
                        _ => "",
                    };
                    if !text.is_empty() {
                        self.put(index(&key(at, "Attributes"), i), text, Some(GREY));
                    }
                }
            }
        }
    }

    fn console_player(&mut self, c: &Compound, at: &TagPath) {
        self.xp(c, at, "The level shown on the XP bar");
        if let Some(Tag::Int(v)) = c.get("Dimension") {
            let named = match v {
                0 => Some(("Overworld", "lime")),
                1 => Some(("Nether", "red")),
                2 => Some(("The End", "magenta")),
                _ => None,
            };
            if let Some((n, col)) = named {
                self.put(key(at, "Dimension"), n, Some(col));
            }
        }
        self.common_player(c, at);
        self.food(c, at);
    }

    // --------------------------------------------------------------- what any entity shows

    /// The player is shown like an entity: a picture, a name, where it is and which way it faces.
    fn entity(&mut self, c: &Compound, at: &TagPath) {
        let custom = custom_name(c);
        let mut text = "Player".to_string();
        if !custom.is_empty() {
            text += &format!(" \"{custom}\"");
        }
        self.put(at.clone(), text, Some(GREY));
        self.icon(at.clone(), NoteIcon::Own(Icon::User));

        let bedrock = self.edition == edition::BEDROCK;
        if let Some(Tag::List(pos)) = c.get("Pos") {
            let v: Vec<f64> = pos.iter().filter_map(|t| as_f64(t, bedrock)).collect();
            if v.len() == 3 {
                let r = |x: f64| (x + 0.5).floor() as i64;
                let p = key(at, "Pos");
                self.put(p.clone(), format!("X:{}, Y:{}, Z:{}", r(v[0]), r(v[1]), r(v[2])), Some(GREY));
                self.icon(p.clone(), NoteIcon::Image("POS"));
                for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                    self.put(index(&p, i), axis, None);
                }
            }
        }
        if let Some(Tag::List(rot)) = c.get("Rotation") {
            let v: Vec<f64> = rot.iter().filter_map(|t| as_f64(t, true)).collect();
            if v.len() >= 2 {
                const YAW: [&str; 8] =
                    ["North", "North-West", "West", "South-West", "South", "South-East", "East", "North-East"];
                const PITCH: [&str; 5] = ["Up", "Slighty Up", "Out", "Slighty Down", "Down"];
                let yaw = format!("Facing {}", YAW[(((v[0].rem_euclid(360.0) + 22.5) / 45.0).floor() as i64).rem_euclid(8) as usize]);
                let pitch = format!("Looking {}", PITCH[(((v[1].rem_euclid(180.0) + 60.0) / 30.0).floor() as i64).rem_euclid(5) as usize]);
                let p = key(at, "Rotation");
                self.put(index(&p, 0), format!("Yaw: {yaw}"), None);
                self.put(index(&p, 1), format!("Pitch: {pitch}"), None);
                self.put(p.clone(), format!("{yaw} and {pitch}"), Some(GREY));
                self.icon(p, NoteIcon::Image("ROT"));
            }
        }
        if let Some(Tag::List(motion)) = c.get("Motion") {
            let v: Vec<f64> = motion.iter().filter_map(|t| as_f64(t, false)).collect();
            if v.len() == 3 {
                let p = key(at, "Motion");
                let axes = [("X", "West", "East"), ("Y", "Down", "Up"), ("Z", "South", "North")];
                for (i, (axis, neg, pos)) in axes.into_iter().enumerate() {
                    let mut label = axis.to_string();
                    if v[i] != 0.0 {
                        label += &format!(
                            ": {}m/s {}",
                            trim_zero((v[i].abs() * 20.0) as f32),
                            if v[i] < 0.0 { neg } else { pos }
                        );
                    }
                    self.put(index(&p, i), label, None);
                }
            }
        }
        let health = match c.get("Health") {
            Some(Tag::Short(v)) => Some(*v as f32),
            Some(Tag::Float(v)) => Some(*v),
            _ => None,
        };
        if let Some(h) = health {
            self.put(key(at, "Health"), format!("{} Hearts", trim_zero(h / 2.0)), None);
            self.icon(key(at, "Health"), HEART);
        }
        if let Some(Tag::Short(v)) = c.get("Fire") {
            if *v <= 0 {
                self.put(key(at, "Fire"), "Not on fire", None);
            } else {
                self.put(key(at, "Fire"), format!("Fire extinguishes in {}", ticks_to_time(*v as f64, true)), None);
                self.icon(key(at, "Fire"), FIRE);
            }
        }
        if let Some(Tag::Short(v)) = c.get("Air") {
            if *v <= 0 {
                self.put(key(at, "Air"), "Drowning", None);
            } else {
                self.put(key(at, "Air"), format!("{} of air", ticks_to_time(*v as f64, true)), None);
            }
        }
        if let Some(Tag::Int(v)) = c.get("PortalCooldown") {
            if *v > 0 {
                self.put(key(at, "PortalCooldown"), ticks_to_time(*v as f64, true), None);
            }
        }
        if let Some(Tag::Float(v)) = c.get("AbsorptionAmount") {
            if *v > 0.0 {
                self.put(key(at, "AbsorptionAmount"), format!("{} Hearts", trim_zero(v / 2.0)), None);
                self.icon(key(at, "AbsorptionAmount"), GOLD_HEART);
            }
        }
        if let Some(Tag::List(effects)) = c.get("ActiveEffects") {
            for (i, e) in effects.iter().enumerate() {
                if let Some(Tag::Int(d)) = compound(e).and_then(|e| e.get("Duration")) {
                    if *d >= 0 {
                        let p = index(&key(at, "ActiveEffects"), i);
                        self.put(key(&p, "Duration"), ticks_to_time(*d as f64, true), None);
                    }
                }
            }
        }
    }
}

/// A list element as a number: doubles in Java, floats in Bedrock (`float` says which is wanted).
fn as_f64(t: &Tag, float: bool) -> Option<f64> {
    match (t, float) {
        (Tag::Double(v), false) => Some(*v),
        (Tag::Float(v), true) => Some(*v as f64),
        (Tag::Double(v), true) => Some(*v),
        (Tag::Float(v), false) => Some(*v as f64),
        _ => None,
    }
}

/// A name given to an entity, which Java may store as JSON text (`{"text":"x","extra":[...]}`).
fn custom_name(c: &Compound) -> String {
    let Some(Tag::String(name)) = c.get("CustomName") else {
        return String::new();
    };
    match serde_json::from_str::<serde_json::Value>(name) {
        Ok(serde_json::Value::Object(o)) => {
            let mut out = o.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string();
            if let Some(extra) = o.get("extra").and_then(|e| e.as_array()) {
                for part in extra {
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        out += t;
                    }
                }
            }
            out
        }
        _ => name.clone(),
    }
}

/// A number with one decimal, and no ".0" after a whole one.
fn trim_zero(v: f32) -> String {
    format!("{v:.1}").replace(".0", "")
}

/// Ticks (20 a second) as days, hours, minutes and seconds. `plural_aware` writes "1 day".
fn ticks_to_time(ticks: f64, plural_aware: bool) -> String {
    let secs = ticks / 20.0;
    let mins = (secs.floor() as i64) / 60;
    let hours = mins / 60;
    let days = hours / 24;
    let unit = |n: i64, one: &str, many: &str| if plural_aware && n == 1 { one.to_string() } else { many.to_string() };
    let mut out = String::new();
    if days > 0 {
        out += &format!("{days}{}", unit(days, " day, ", " days, "));
    }
    if hours > 0 {
        out += &format!("{}{}", hours % 24, unit(hours % 24, " hour, ", " hours, "));
    }
    if mins > 0 {
        out += &format!("{}{}", mins % 60, unit(mins % 60, " minute, ", " minutes, "));
    }
    if secs > 0.0 {
        let s = secs.rem_euclid(60.0);
        let text = if plural_aware {
            let t = format!("{s:.2}");
            let t = t.trim_end_matches('0').trim_end_matches('.');
            t.to_string()
        } else {
            trim_zero(s as f32)
        };
        out += &format!("{text}{}", if plural_aware && s <= 1.0 { " second" } else { " seconds" });
    }
    out
}

/// Whole seconds as days, hours, minutes and seconds.
fn long_time(secs: i64) -> String {
    let (mins, hours) = (secs / 60, secs / 3600);
    let days = hours / 24;
    let mut out = String::new();
    if days > 0 {
        out += &format!("{days} days, ");
    }
    if hours > 0 {
        out += &format!("{} hours, ", hours % 24);
    }
    if mins > 0 {
        out += &format!("{} minutes, ", mins % 60);
    }
    if secs > 0 {
        out += &format!("{} seconds", secs % 60);
    }
    out
}

/// The time of day a Java `DayTime` tick count is (0 is 6 AM).
fn day_time(ticks: i64) -> String {
    let t = ticks % 24000;
    let mut hour = (t / 1000) + 6;
    if hour >= 24 {
        hour -= 24;
    }
    let mut ampm = "AM";
    if hour >= 12 {
        ampm = "PM";
        hour -= 12;
    }
    if hour == 0 {
        hour = 12;
    }
    let part = if t > 3000 && t <= 9000 {
        "Noon"
    } else if t > 9000 && t <= 15000 {
        "Sunset"
    } else if t > 15000 && t <= 21000 {
        "Midnight"
    } else {
        "Sunrise"
    };
    format!("{part} {hour}{ampm}")
}

/// A time in seconds since 1970 as "October 29, 2026, 03:36AM" (UTC).
fn format_time(secs: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Days since 1970 as a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    let (h, m) = (rem / 3600, (rem % 3600) / 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{} {day:02}, {year}, {h12:02}:{m:02}{ampm}", MONTHS[(month - 1) as usize])
}

/// The name of the game version a `DataVersion` belongs to, for those the table knows.
fn data_version_name(v: i32) -> Option<String> {
    static TABLE: std::sync::OnceLock<HashMap<i32, &'static str>> = std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| {
            // Our own registries record the DataVersion of every release they cover.
            let mut t = HashMap::new();
            for (dv, name) in mctk_registry::builtin::java_data_versions() {
                t.entry(dv).or_insert(name);
            }
            t
        })
        .get(&v)
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_naturally() {
        assert_eq!(ticks_to_time(20.0 * 90.0, true), "1 minute, 30 seconds");
        assert_eq!(long_time(3_700), "1 hours, 1 minutes, 40 seconds");
        assert_eq!(day_time(6000), "Noon 12PM");
        assert_eq!(format_time(1_793_245_010), "October 29, 2026, 03:36AM");
    }

    #[test]
    fn a_player_gets_a_name_a_picture_and_notes() {
        let mut c = Compound::new();
        c.insert("playerGameType".into(), Tag::Int(1));
        c.insert("foodLevel".into(), Tag::Int(20));
        let root = Tag::Compound(c);
        let s = style_tree(&root, edition::JAVA, file_type::PLAYER);
        let mode = &s[&vec![Seg::Key("playerGameType".into())]];
        assert_eq!(mode.label.as_deref(), Some("Creative"));
        let food = &s[&vec![Seg::Key("foodLevel".into())]];
        assert_eq!(food.label.as_deref(), Some("10 Food (Full)"));
        assert_eq!(s[&TagPath::new()].icon, Some(NoteIcon::Own(Icon::User)));
    }

    #[test]
    fn data_versions_are_named() {
        assert_eq!(data_version_name(3463).as_deref(), Some("1.20"));
    }
}
