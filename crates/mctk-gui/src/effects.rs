//! Status effects: on mobs and players, and the custom effects of potions (and tipped arrows),
//! in each edition's layout, with the menu that adds and changes them; and a potion's kind
//! (to drink, splash or lingering).
//!
//! | | where | an effect |
//! |---|---|---|
//! | Java 1.20.2 on (mobs) | `active_effects` | `{id: "minecraft:speed", amplifier, duration}` |
//! | Java before, console (mobs) | `ActiveEffects` | `{Id: 1, Amplifier, Duration}` |
//! | Bedrock (mobs) | `mobEffects` | `{Id: 1, Amplifier, Duration, DurationEasy...}` (Bedrock's numbers) |
//! | Java 1.20.5 on (potions) | `components."minecraft:potion_contents".custom_effects` | as mobs' |
//! | Java 1.20.2 to 1.20.4 (potions) | `tag.custom_potion_effects` | as mobs' |
//! | Java before, console (potions) | `tag.CustomPotionEffects` | `{Id: 1, Amplifier, Duration}` |
//!
//! Bedrock keeps no custom effects on potions (only which potion it is).

use mctk_nbt::{Compound, List, Tag, TagType};

/// A status effect: its Java name (`speed`), its level from 0, and how many ticks are left
/// (-1: for ever).
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub id: String,
    pub amplifier: i64,
    pub duration: i64,
}

/// Effect names by their number: Java's before 1.20.2 and the console editions'.
const JAVA_NUMBERED: [&str; 33] = [
    "speed", "slowness", "haste", "mining_fatigue", "strength", "instant_health", "instant_damage", "jump_boost", "nausea", "regeneration", "resistance",
    "fire_resistance", "water_breathing", "invisibility", "blindness", "night_vision", "hunger", "weakness", "poison", "wither", "health_boost", "absorption",
    "saturation", "glowing", "levitation", "luck", "unluck", "slow_falling", "conduit_power", "dolphins_grace", "bad_omen", "hero_of_the_village", "darkness",
];

/// Bedrock's numbers, which part from Java's after saturation.
const BEDROCK_NUMBERED: [&str; 36] = [
    "speed", "slowness", "haste", "mining_fatigue", "strength", "instant_health", "instant_damage", "jump_boost", "nausea", "regeneration", "resistance",
    "fire_resistance", "water_breathing", "invisibility", "blindness", "night_vision", "hunger", "weakness", "poison", "wither", "health_boost", "absorption",
    "saturation", "levitation", "fatal_poison", "conduit_power", "slow_falling", "bad_omen", "hero_of_the_village", "darkness", "trial_omen", "wind_charged",
    "weaving", "oozing", "infested", "raid_omen",
];

/// Java's effects since 1.20.5, named.
const JAVA_NAMED: [&str; 39] = [
    "speed", "slowness", "haste", "mining_fatigue", "strength", "instant_health", "instant_damage", "jump_boost", "nausea", "regeneration", "resistance",
    "fire_resistance", "water_breathing", "invisibility", "blindness", "night_vision", "hunger", "weakness", "poison", "wither", "health_boost", "absorption",
    "saturation", "glowing", "levitation", "luck", "unluck", "slow_falling", "conduit_power", "dolphins_grace", "bad_omen", "hero_of_the_village", "darkness",
    "trial_omen", "raid_omen", "wind_charged", "weaving", "oozing", "infested",
];

/// Java 1.20.2 (data version 3578), where effects are named; 1.20.5 (3837), where items have
/// components.
const DV_NAMED: i64 = 3578;
const DV_COMPONENTS: i64 = 3837;

/// What the effects are on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Holder {
    /// A mob or a player.
    Mob,
    /// A potion, splash or lingering potion, or tipped arrow (an item stack).
    Potion,
}

/// How a list of effects is kept: the keys from the thing down to it, and how an effect in it
/// is written.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Style {
    /// `{id: "minecraft:speed", amplifier, duration}`
    Named,
    /// `{Id: 1, Amplifier, Duration}`, Java's numbers.
    Numbered,
    /// `{Id: 1, Amplifier, Duration, DurationEasy...}`, Bedrock's numbers.
    Bedrock,
}

const SPOTS: [(Holder, &[&str], Style); 6] = [
    (Holder::Mob, &["active_effects"], Style::Named),
    (Holder::Mob, &["ActiveEffects"], Style::Numbered),
    (Holder::Mob, &["mobEffects"], Style::Bedrock),
    (Holder::Potion, &["components", "minecraft:potion_contents", "custom_effects"], Style::Named),
    (Holder::Potion, &["tag", "custom_potion_effects"], Style::Named),
    (Holder::Potion, &["tag", "CustomPotionEffects"], Style::Numbered),
];

fn is_bedrock_mob(c: &Compound) -> bool {
    c.contains_key("identifier")
}

fn is_bedrock_stack(c: &Compound) -> bool {
    c.contains_key("Name") && !c.contains_key("id")
}

fn at<'a>(c: &'a Compound, keys: &[&str]) -> Option<&'a Tag> {
    let (last, path) = keys.split_last()?;
    let mut here = c;
    for k in path {
        here = here.get(*k)?.as_compound()?;
    }
    here.get(*last)
}

/// Where a thing keeps its effects, and how: where it has some already, else by its edition and
/// version. `None` where its edition keeps none (Bedrock potions).
fn spot(c: &Compound, holder: Holder, data_version: Option<i64>) -> Option<(&'static [&'static str], Style)> {
    let mine = || SPOTS.iter().filter(move |(h, ..)| *h == holder);
    if let Some((_, keys, style)) = mine().find(|(_, keys, _)| matches!(at(c, keys), Some(Tag::List(_)))) {
        return Some((keys, *style));
    }
    let pick = |k: usize| mine().nth(k).map(|(_, keys, style)| (*keys, *style));
    match holder {
        Holder::Mob => {
            if is_bedrock_mob(c) {
                return pick(2);
            }
            let named = match data_version {
                Some(v) => v >= DV_NAMED,
                None => crate::mob_edit::StackForm::of(c, None) == crate::mob_edit::StackForm::Components,
            };
            pick(if named { 0 } else { 1 })
        }
        Holder::Potion => {
            if is_bedrock_stack(c) {
                return None;
            }
            let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_)));
            if c.contains_key("components") || c.contains_key("count") || data_version.is_some_and(|v| v >= DV_COMPONENTS) {
                pick(0)
            } else if !numeric && data_version.is_some_and(|v| v >= DV_NAMED) {
                pick(1)
            } else {
                pick(2)
            }
        }
    }
}

/// What a potion (or tipped arrow) stack does, as lines for a card: which potion it is
/// ("Strong Healing"), then each effect added to it with its time ("Speed II  1:30").
pub fn potion_lines(stack: &Compound) -> Vec<String> {
    let mut out = Vec::new();
    let base = at(stack, &["components", "minecraft:potion_contents", "potion"])
        .or_else(|| at(stack, &["tag", "Potion"]))
        .and_then(Tag::as_str)
        .map(|s| s.trim_start_matches("minecraft:").to_string());
    if let Some(b) = base.filter(|b| !b.is_empty() && b != "empty") {
        out.push(display(&b));
    }
    for e in read(stack, Holder::Potion) {
        out.push(format!("{}  {}", effect_name(&e), time_text(e.duration)));
    }
    out
}

/// The effects on a thing, in any edition's layout. Every entry is there, in order (one it
/// cannot read is `unknown`), so places in the list match the stored list's.
pub fn read(c: &Compound, holder: Holder) -> Vec<Effect> {
    let bedrock = is_bedrock_mob(c);
    let Some((_, keys, _)) = SPOTS.iter().find(|(h, keys, _)| *h == holder && matches!(at(c, keys), Some(Tag::List(_)))) else {
        return Vec::new();
    };
    let Some(Tag::List(list)) = at(c, keys) else { return Vec::new() };
    list.iter()
        .map(|t| {
            let e = t.as_compound();
            let get = |a: &str, b: &str| e.and_then(|e| e.get(a).or_else(|| e.get(b)));
            let id = match get("id", "Id") {
                Some(Tag::String(s)) => s.trim_start_matches("minecraft:").to_string(),
                Some(t) => {
                    let names: &[&str] = if bedrock { &BEDROCK_NUMBERED } else { &JAVA_NUMBERED };
                    t.as_i64().and_then(|n| usize::try_from(n - 1).ok()).and_then(|n| names.get(n)).map_or_else(|| "unknown".to_string(), |s| (*s).to_string())
                }
                None => "unknown".to_string(),
            };
            let amplifier = get("amplifier", "Amplifier").and_then(Tag::as_i64).unwrap_or(0);
            let duration = get("duration", "Duration").and_then(Tag::as_i64).unwrap_or(0);
            Effect { id, amplifier, duration }
        })
        .collect()
}

/// The effects a thing can be given: those its layout has names or numbers for.
pub fn choices(c: &Compound, holder: Holder, data_version: Option<i64>) -> Vec<&'static str> {
    match spot(c, holder, data_version) {
        None => Vec::new(),
        Some((_, Style::Named)) => JAVA_NAMED.to_vec(),
        Some((_, Style::Numbered)) => JAVA_NUMBERED.to_vec(),
        Some((_, Style::Bedrock)) => BEDROCK_NUMBERED.to_vec(),
    }
}

/// One change to a thing's effects.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectEdit {
    /// A new effect (level I, 30 seconds), after the others.
    Add(String),
    /// The `n`th effect's level, from 0.
    Amplifier(usize, i64),
    /// The `n`th effect's time left in ticks (-1: for ever).
    Duration(usize, i64),
    Remove(usize),
}

impl EffectEdit {
    pub fn note(&self) -> String {
        match self {
            EffectEdit::Add(id) => format!("Gave it {}", crate::item_picker::display_name(id)),
            EffectEdit::Amplifier(_, a) => format!("Set an effect to level {}", a + 1),
            EffectEdit::Duration(_, d) if *d < 0 => "Made an effect last for ever".into(),
            EffectEdit::Duration(_, d) => format!("Set an effect to {}", time_text(*d)),
            EffectEdit::Remove(_) => "Took an effect away".into(),
        }
    }

    pub fn apply(&self, c: &mut Compound, holder: Holder, data_version: Option<i64>) {
        let Some((keys, style)) = spot(c, holder, data_version) else { return };
        // The list, made where it is missing (a potion's contents saved as just its name become
        // a compound with that name).
        let mut here: &mut Compound = c;
        let (last, path) = keys.split_last().expect("keys");
        for k in path {
            let slot = here.entry((*k).to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
            if let Tag::String(name) = slot {
                let mut contents = Compound::new();
                contents.insert("potion".into(), Tag::String(name.clone()));
                *slot = Tag::Compound(contents);
            }
            let Tag::Compound(next) = slot else { return };
            here = next;
        }
        let list = here.entry((*last).to_string()).or_insert_with(|| Tag::List(List::new(TagType::Compound)));
        let Tag::List(list) = list else { return };
        let mut items: Vec<Tag> = list.iter().cloned().collect();
        let set = |e: &mut Tag, named: &str, numbered: &str, v: Tag| {
            if let Some(e) = e.as_compound_mut() {
                let key = if e.contains_key(named) || style == Style::Named { named } else { numbered };
                e.insert(key.into(), v);
            }
        };
        match self {
            EffectEdit::Add(id) => items.push(Tag::Compound(make(style, id, 0, 600))),
            EffectEdit::Amplifier(n, a) => {
                if let Some(e) = items.get_mut(*n) {
                    set(e, "amplifier", "Amplifier", Tag::Byte((*a).clamp(0, 255) as u8 as i8));
                }
            }
            EffectEdit::Duration(n, d) => {
                if let Some(e) = items.get_mut(*n) {
                    let d = Tag::Int((*d).clamp(-1, i64::from(i32::MAX)) as i32);
                    set(e, "duration", "Duration", d.clone());
                    if style == Style::Bedrock {
                        if let Some(e) = e.as_compound_mut() {
                            for k in ["DurationEasy", "DurationNormal", "DurationHard"] {
                                e.insert(k.into(), d.clone());
                            }
                        }
                    }
                }
            }
            EffectEdit::Remove(n) => {
                if *n < items.len() {
                    items.remove(*n);
                }
            }
        }
        if items.is_empty() {
            here.shift_remove(*last);
            return;
        }
        if let Ok(l) = List::from_tags(TagType::Compound, items) {
            here.insert((*last).to_string(), Tag::List(l));
        }
    }
}

/// A new effect in a style.
fn make(style: Style, id: &str, amplifier: i64, duration: i64) -> Compound {
    let mut e = Compound::new();
    let bare = id.trim_start_matches("minecraft:");
    match style {
        Style::Named => {
            e.insert("id".into(), Tag::String(format!("minecraft:{bare}")));
            e.insert("amplifier".into(), Tag::Byte(amplifier as i8));
            e.insert("duration".into(), Tag::Int(duration as i32));
            e.insert("ambient".into(), Tag::Byte(0));
            e.insert("show_particles".into(), Tag::Byte(1));
            e.insert("show_icon".into(), Tag::Byte(1));
        }
        Style::Numbered | Style::Bedrock => {
            let names: &[&str] = if style == Style::Bedrock { &BEDROCK_NUMBERED } else { &JAVA_NUMBERED };
            let n = names.iter().position(|x| *x == bare).map_or(1, |k| k + 1);
            e.insert("Id".into(), Tag::Byte(n as i8));
            e.insert("Amplifier".into(), Tag::Byte(amplifier as i8));
            e.insert("Duration".into(), Tag::Int(duration as i32));
            e.insert("Ambient".into(), Tag::Byte(0));
            e.insert("ShowParticles".into(), Tag::Byte(1));
            if style == Style::Bedrock {
                for k in ["DurationEasy", "DurationNormal", "DurationHard"] {
                    e.insert(k.into(), Tag::Int(duration as i32));
                }
                e.insert("DisplayOnScreenTextureAnimation".into(), Tag::Byte(0));
            } else {
                e.insert("ShowIcon".into(), Tag::Byte(1));
            }
        }
    }
    e
}

/// An effect for a lingering cloud, in the layout of a Java world of `data_version`: named from
/// 1.20.2 on, numbered before. Effects with no number in the old layout are given as speed.
pub fn cloud_effect(data_version: i64, id: &str, amplifier: i64, duration: i64) -> Compound {
    make(if data_version >= DV_NAMED { Style::Named } else { Style::Numbered }, id, amplifier, duration)
}

/// The name of Java's effect with a number (from 1), as worlds before 1.20.2 keep them.
pub fn java_effect_name(n: i64) -> Option<&'static str> {
    usize::try_from(n - 1).ok().and_then(|k| JAVA_NUMBERED.get(k)).copied()
}

/// The Java effects that can be given a cloud (named; before 1.20.2 only the first 33 have numbers).
pub fn cloud_effect_names() -> &'static [&'static str] {
    &JAVA_NAMED
}

/// Ticks as the game shows time left: "1:30", "1:00:00", "∞".
pub fn time_text(ticks: i64) -> String {
    if ticks < 0 {
        return "\u{221e}".into();
    }
    let s = ticks / 20;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// An effect's name and level as the game shows it: "Speed II".
pub fn effect_name(e: &Effect) -> String {
    const ROMAN: [&str; 10] = ["", " II", " III", " IV", " V", " VI", " VII", " VIII", " IX", " X"];
    let level = usize::try_from(e.amplifier).ok().and_then(|a| ROMAN.get(a)).map_or_else(|| format!(" {}", e.amplifier + 1), |s| (*s).to_string());
    format!("{}{level}", display(&e.id))
}

/// "Fatal Poison", "Hero Of The Village".
fn display(id: &str) -> String {
    crate::item_picker::display_name(id)
}

/// The game's picture of an effect (fatal poison is drawn as poison).
pub fn icon_file(id: &str) -> Option<std::path::PathBuf> {
    let id = if id == "fatal_poison" { "poison" } else { id };
    crate::icons::pack_texture(&format!("mob_effect/{id}.png"))
}

// ---------------------------------------------------------------------------------------------
// Potions
// ---------------------------------------------------------------------------------------------

/// Whether a stack (by its Java id) holds effects: potions of each kind, tipped arrows.
pub fn holds_effects(id: &str) -> bool {
    matches!(id.trim_start_matches("minecraft:"), "potion" | "splash_potion" | "lingering_potion" | "tipped_arrow")
}

/// The kinds of potion: to drink, splash, lingering.
pub const KINDS: [(&str, &str); 3] = [("minecraft:potion", "Drinkable"), ("minecraft:splash_potion", "Splash"), ("minecraft:lingering_potion", "Lingering")];

/// Old Java and the console editions keep one potion item, splash when its damage has this bit.
const SPLASH_BIT: i64 = 0x4000;

/// Makes a potion stack of another kind, keeping everything else on it.
pub fn set_kind(c: &mut Compound, kind: &str) {
    match c.get("id").cloned() {
        Some(Tag::String(_)) => {
            c.insert("id".into(), Tag::String(kind.to_string()));
        }
        Some(Tag::Short(_) | Tag::Int(_)) => {
            // A world with an item of its own for each kind uses it; else the splash bit.
            let legacy = mctk_mapping::legacy::legacy();
            let damage = c.get("Damage").and_then(Tag::as_i64).unwrap_or(0);
            match legacy.numeric_item(kind).filter(|(n, _)| *n != 373) {
                Some((n, _)) => {
                    let is_int = matches!(c.get("id"), Some(Tag::Int(_)));
                    c.insert("id".into(), if is_int { Tag::Int(i32::from(n)) } else { Tag::Short(n as i16) });
                }
                None => {
                    let splash = kind.ends_with("splash_potion");
                    let d = if splash { damage | SPLASH_BIT } else { damage & !SPLASH_BIT };
                    c.insert("id".into(), Tag::Short(373));
                    c.insert("Damage".into(), Tag::Short(d as i16));
                }
            }
        }
        _ => {
            if c.contains_key("Name") {
                c.insert("Name".into(), Tag::String(kind.to_string()));
            }
        }
    }
}

/// A potion's kind (its id, as in [`KINDS`]).
pub fn kind_of(id: &str, c: &Compound) -> &'static str {
    let bare = id.trim_start_matches("minecraft:");
    if bare == "splash_potion" || c.get("Damage").and_then(Tag::as_i64).is_some_and(|d| matches!(c.get("id"), Some(Tag::Short(373) | Tag::Int(373))) && d & SPLASH_BIT != 0) {
        KINDS[1].0
    } else if bare == "lingering_potion" {
        KINDS[2].0
    } else {
        KINDS[0].0
    }
}

/// A change to a potion from its menu.
#[derive(Clone, Debug, PartialEq)]
pub enum PotionEdit {
    Kind(String),
    Effect(EffectEdit),
}

impl PotionEdit {
    pub fn note(&self) -> String {
        match self {
            PotionEdit::Kind(k) => format!("Made it a {} potion", KINDS.iter().find(|(id, _)| id == k).map_or("", |(_, n)| n).to_lowercase()),
            PotionEdit::Effect(e) => e.note(),
        }
    }

    pub fn apply(&self, c: &mut Compound, data_version: Option<i64>) {
        match self {
            PotionEdit::Kind(k) => set_kind(c, k),
            PotionEdit::Effect(e) => e.apply(c, Holder::Potion, data_version),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The menus
// ---------------------------------------------------------------------------------------------

/// Paints an effect's picture in gray (the menus' rows), or in color (the list to add from).
pub fn paint_effect(p: &eframe::egui::Painter, r: eframe::egui::Rect, id: &str, gray: bool, ink: eframe::egui::Color32) {
    let drawn = icon_file(id).is_some_and(|f| if gray { crate::icons::gray_image(p, r, &f) } else { crate::icons::file_image(p, r, &f) });
    if !drawn {
        p.circle_stroke(r.center(), r.width() * 0.35, eframe::egui::Stroke::new(1.3_f32, ink));
    }
}

/// `Status Effects ▸`: each effect (its level, time left and removing it, in a menu of its own)
/// and adding one from `choices`. `key` keeps what is typed apart from other things' menus.
pub fn menu(ui: &mut eframe::egui::Ui, cx: &mut crate::ctx_menu::Cx, key: &str, label: &str, effects: &[Effect], choices: &[&'static str]) -> Option<EffectEdit> {
    use crate::ctx_menu;
    use crate::icons::Icon;
    if choices.is_empty() {
        return None;
    }
    let height = 60.0 + (effects.len() as f32 + 1.0) * 32.0;
    ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("potion"), label, 300.0, height, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 300.0);
        let mut inner = ctx_menu::Cx::default();
        let mut out = None;
        if effects.is_empty() {
            ui.label(eframe::egui::RichText::new("No effects").color(crate::theme::text_dim()));
        }
        for (i, e) in effects.iter().enumerate() {
            let row = format!("{}  {}", effect_name(e), time_text(e.duration));
            let id = e.id.clone();
            let chosen = ctx_menu::submenu_painted(ui, &mut inner, move |p, r, ink| paint_effect(p, r, &id, true, ink), &row, 240.0, 160.0, |ui| {
                let old = ctx_menu::set_row_width(ui.ctx(), 240.0);
                let mut out = None;
                // Its level (I is 0 as stored) and its time in seconds, typed or dragged.
                let lk = eframe::egui::Id::new(("effect-level", key, i));
                let mut level: i64 = ui.ctx().data(|d| d.get_temp(lk)).unwrap_or(e.amplifier + 1);
                let (changed, done) = ctx_menu::number_row(ui, Icon::Rename, "Level", &mut level, 1..=256, "");
                if changed {
                    ui.ctx().data_mut(|d| d.insert_temp(lk, level));
                }
                if done {
                    ui.ctx().data_mut(|d| d.remove_temp::<i64>(lk));
                    if level - 1 != e.amplifier {
                        out = Some(EffectEdit::Amplifier(i, level - 1));
                    }
                }
                let forever = e.duration < 0;
                if !forever {
                    let sk = eframe::egui::Id::new(("effect-seconds", key, i));
                    let now = e.duration as f64 / 20.0;
                    let mut secs: f64 = ui.ctx().data(|d| d.get_temp(sk)).unwrap_or(now);
                    let (changed, done) = ctx_menu::decimal_row(ui, Icon::Rename, "Seconds", &mut secs, 0.0..=100_000_000.0);
                    if changed {
                        ui.ctx().data_mut(|d| d.insert_temp(sk, secs));
                    }
                    if done {
                        ui.ctx().data_mut(|d| d.remove_temp::<f64>(sk));
                        let ticks = (secs * 20.0).round() as i64;
                        if ticks != e.duration {
                            out = Some(EffectEdit::Duration(i, ticks));
                        }
                    }
                }
                if ctx_menu::toggle(ui, "Infinite", forever) {
                    out = Some(EffectEdit::Duration(i, if forever { 600 } else { -1 }));
                }
                ctx_menu::separator(ui);
                if ctx_menu::item(ui, Icon::Trash, "Remove", "", true) {
                    out = Some(EffectEdit::Remove(i));
                }
                ctx_menu::set_row_width(ui.ctx(), old);
                out
            });
            if chosen.is_some() {
                out = chosen;
            }
        }
        ctx_menu::separator(ui);
        let added = ctx_menu::submenu_wide(ui, &mut inner, Icon::Plus, "Add Effect", 240.0, 360.0, |ui| {
            let old = ctx_menu::set_row_width(ui.ctx(), 240.0);
            let mut out = None;
            eframe::egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                for id in choices {
                    if ctx_menu::painted(ui, &display(id), "", true, |p, r, ink| paint_effect(p, r, id, false, ink)) {
                        out = Some(EffectEdit::Add((*id).to_string()));
                    }
                }
            });
            ctx_menu::set_row_width(ui.ctx(), old);
            out
        });
        ctx_menu::set_row_width(ui.ctx(), old);
        added.or(out)
    })
}

/// `Potion ▸` for a potion stack (`id` its Java id): its kind, and its custom effects. Tipped
/// arrows have only the effects.
pub fn potion_menu(ui: &mut eframe::egui::Ui, cx: &mut crate::ctx_menu::Cx, key: &str, id: &str, stack: &Compound, data_version: Option<i64>) -> Option<PotionEdit> {
    use crate::ctx_menu;
    if !holds_effects(id) {
        return None;
    }
    let choices = choices(stack, Holder::Potion, data_version);
    let effects = read(stack, Holder::Potion);
    let arrow = id.ends_with("tipped_arrow");
    if arrow {
        return menu(ui, cx, key, "Custom Effects", &effects, &choices).map(PotionEdit::Effect);
    }
    let now = kind_of(id, stack);
    // Lingering potions came with Java 1.9: a world that numbers its items may have none.
    let numeric = matches!(stack.get("id"), Some(Tag::Short(_) | Tag::Int(_)));
    let lingering_ok = !numeric || mctk_mapping::legacy::legacy().numeric_item("minecraft:lingering_potion").is_some_and(|(n, _)| n != 373);
    let height = 60.0 + 4.0 * 32.0;
    ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("splash_potion"), "Potion", 280.0, height, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 280.0);
        let mut inner = ctx_menu::Cx::default();
        let mut out = None;
        for (kind, name) in KINDS {
            if kind.ends_with("lingering_potion") && !lingering_ok {
                continue;
            }
            if ctx_menu::toggle(ui, name, now == kind) && now != kind {
                out = Some(PotionEdit::Kind(kind.to_string()));
            }
        }
        ctx_menu::separator(ui);
        if choices.is_empty() {
            ui.label(eframe::egui::RichText::new("Bedrock keeps no custom effects on potions").color(crate::theme::text_dim()));
        } else if let Some(e) = menu(ui, &mut inner, key, "Custom Effects", &effects, &choices) {
            out = Some(PotionEdit::Effect(e));
        }
        ctx_menu::set_row_width(ui.ctx(), old);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }
    fn of(pairs: &[(&str, Tag)]) -> Compound {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn mobs_keep_effects_in_their_editions_layout() {
        // Java 1.21: named.
        let mut c = of(&[("id", s("minecraft:zombie")), ("Health", Tag::Float(20.0))]);
        EffectEdit::Add("speed".into()).apply(&mut c, Holder::Mob, Some(4000));
        EffectEdit::Amplifier(0, 1).apply(&mut c, Holder::Mob, Some(4000));
        EffectEdit::Duration(0, -1).apply(&mut c, Holder::Mob, Some(4000));
        let e = c.get("active_effects").and_then(Tag::as_list).and_then(|l| l.get(0)).and_then(Tag::as_compound).unwrap();
        assert_eq!(e.get("id"), Some(&s("minecraft:speed")));
        assert_eq!(read(&c, Holder::Mob), vec![Effect { id: "speed".into(), amplifier: 1, duration: -1 }]);
        EffectEdit::Remove(0).apply(&mut c, Holder::Mob, Some(4000));
        assert!(!c.contains_key("active_effects"));
        // Java 1.12: by number.
        let mut c = of(&[("id", s("minecraft:zombie")), ("Health", Tag::Float(20.0))]);
        EffectEdit::Add("glowing".into()).apply(&mut c, Holder::Mob, Some(1343));
        let e = c.get("ActiveEffects").and_then(Tag::as_list).and_then(|l| l.get(0)).and_then(Tag::as_compound).unwrap();
        assert_eq!(e.get("Id"), Some(&Tag::Byte(24)));
        // Bedrock: Bedrock's numbers, every difficulty's time.
        let mut b = of(&[("identifier", s("minecraft:zombie"))]);
        EffectEdit::Add("levitation".into()).apply(&mut b, Holder::Mob, None);
        EffectEdit::Duration(0, 200).apply(&mut b, Holder::Mob, None);
        let e = b.get("mobEffects").and_then(Tag::as_list).and_then(|l| l.get(0)).and_then(Tag::as_compound).unwrap();
        assert_eq!((e.get("Id"), e.get("DurationHard")), (Some(&Tag::Byte(24)), Some(&Tag::Int(200))));
        assert_eq!(read(&b, Holder::Mob)[0].id, "levitation");
    }

    #[test]
    fn potions_keep_effects_and_change_kind() {
        // Java 1.21: the potion's contents, saved as its name, become a compound keeping it.
        let mut c = of(&[("id", s("minecraft:potion")), ("count", Tag::Int(1))]);
        let mut comps = Compound::new();
        comps.insert("minecraft:potion_contents".into(), s("minecraft:water"));
        c.insert("components".into(), Tag::Compound(comps));
        EffectEdit::Add("speed".into()).apply(&mut c, Holder::Potion, Some(4000));
        let contents = c.get("components").and_then(|t| t.get("minecraft:potion_contents")).and_then(Tag::as_compound).unwrap();
        assert_eq!(contents.get("potion"), Some(&s("minecraft:water")));
        assert_eq!(read(&c, Holder::Potion).len(), 1);
        PotionEdit::Kind("minecraft:lingering_potion".into()).apply(&mut c, Some(4000));
        assert_eq!(c.get("id"), Some(&s("minecraft:lingering_potion")));
        assert_eq!(read(&c, Holder::Potion).len(), 1, "the effects stay");
        // Java 1.12: in `tag`, by number.
        let mut c = of(&[("id", s("minecraft:potion")), ("Count", Tag::Byte(1))]);
        EffectEdit::Add("speed".into()).apply(&mut c, Holder::Potion, Some(1343));
        assert!(c.get("tag").and_then(|t| t.get("CustomPotionEffects")).is_some());
        // Bedrock: no custom effects.
        let b = of(&[("Name", s("minecraft:potion")), ("Count", Tag::Byte(1))]);
        assert!(choices(&b, Holder::Potion, None).is_empty());
    }
}
