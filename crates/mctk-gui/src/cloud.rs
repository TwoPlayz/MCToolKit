//! Lingering potion clouds (the game's `area_effect_cloud` entity): what the user chooses for
//! one, how it is written for each version of a Java world, and the colors its particles take.
//!
//! | Java | effects | particles hidden by |
//! |---|---|---|
//! | 1.20.5 on | `potion_contents.custom_effects` | `custom_particle` of an air block |
//! | 1.20.2 to 1.20.4 | `Effects`, effects named | `Particle: "block minecraft:air"` |
//! | before | `Effects`, effects numbered | the same |
//!
//! A block of air spawns no particles, which is how the game's own commands hide a cloud's.

use mctk_nbt::{Compound, List, Tag, TagType};

/// One effect of a cloud: its Java name (`speed`), level from 1, and seconds (or for ever).
#[derive(Clone, Debug, PartialEq)]
pub struct CloudEffect {
    pub id: String,
    pub level: i64,
    pub seconds: i64,
    /// Lasts for ever on whoever it touches (a duration of -1).
    pub forever: bool,
}

impl CloudEffect {
    pub fn new(id: &str) -> Self {
        CloudEffect { id: id.trim_start_matches("minecraft:").to_string(), level: 1, seconds: 30, forever: false }
    }
}

/// What the user chose for a cloud to place.
#[derive(Clone, Debug, PartialEq)]
pub struct Cloud {
    /// Blocks from the middle to the edge.
    pub radius: f32,
    /// How long it lingers, in seconds (unless `forever`).
    pub lasts: i64,
    /// It never goes (its duration is the longest there is).
    pub forever: bool,
    pub effects: Vec<CloudEffect>,
    /// The color of its particles, when not the mix of its effects' colors.
    pub color: Option<[u8; 3]>,
    /// Whether its particles show (the effects work either way).
    pub particles: bool,
    /// Whether it shrinks as it ages and as it is used, as a thrown potion's does.
    pub shrinks: bool,
}

impl Default for Cloud {
    fn default() -> Self {
        Cloud { radius: 3.0, lasts: 30, forever: false, effects: vec![CloudEffect::new("regeneration")], color: None, particles: true, shrinks: true }
    }
}

/// Java 1.20.5 (data version 3837) puts a cloud's effects in `potion_contents`.
const DV_COMPONENTS: i64 = 3837;

/// A random identity for an entity, in the form Java keeps it.
fn uuid() -> Tag {
    use std::hash::{BuildHasher, Hasher};
    let word = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
        h.finish()
    };
    let (a, b) = (word(), word());
    Tag::IntArray(vec![(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32])
}

fn list(kind: TagType, items: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(kind, items).unwrap_or_else(|_| List::new(kind)))
}

fn doubles(v: [f64; 3]) -> Tag {
    list(TagType::Double, v.iter().map(|x| Tag::Double(*x)).collect())
}

impl Cloud {
    /// The cloud as an entity standing at `pos`, written for a world of `data_version`.
    pub fn entity(&self, pos: [f64; 3], data_version: i64) -> Compound {
        // For ever is the longest a cloud can last.
        let ticks = if self.forever { i32::MAX } else { (self.lasts.max(1) * 20) as i32 };
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String("minecraft:area_effect_cloud".into()));
        c.insert("Pos".into(), doubles(pos));
        c.insert("Motion".into(), doubles([0.0; 3]));
        c.insert("Rotation".into(), list(TagType::Float, vec![Tag::Float(0.0), Tag::Float(0.0)]));
        c.insert("UUID".into(), uuid());
        c.insert("Age".into(), Tag::Int(0));
        c.insert("Duration".into(), Tag::Int(ticks));
        c.insert("WaitTime".into(), Tag::Int(10));
        c.insert("ReapplicationDelay".into(), Tag::Int(20));
        c.insert("DurationOnUse".into(), Tag::Int(0));
        c.insert("Radius".into(), Tag::Float(self.radius.max(0.1)));
        // A thrown potion's cloud shrinks to nothing as it ages, and a little with each use.
        c.insert("RadiusOnUse".into(), Tag::Float(if self.shrinks { -0.5 } else { 0.0 }));
        c.insert("RadiusPerTick".into(), Tag::Float(if self.shrinks && !self.forever { -self.radius.max(0.1) / ticks as f32 } else { 0.0 }));
        let effects: Vec<Tag> = self
            .effects
            .iter()
            .map(|e| Tag::Compound(crate::effects::cloud_effect(data_version, &e.id, (e.level - 1).clamp(0, 255), if e.forever { -1 } else { e.seconds.max(1) * 20 })))
            .collect();
        if data_version >= DV_COMPONENTS {
            let mut contents = Compound::new();
            if !effects.is_empty() {
                contents.insert("custom_effects".into(), list(TagType::Compound, effects));
            }
            if let Some(rgb) = self.color {
                contents.insert("custom_color".into(), Tag::Int(rgb_int(rgb)));
            }
            c.insert("potion_contents".into(), Tag::Compound(contents));
            if !self.particles {
                let mut p = Compound::new();
                p.insert("type".into(), Tag::String("minecraft:block".into()));
                let mut state = Compound::new();
                state.insert("Name".into(), Tag::String("minecraft:air".into()));
                p.insert("block_state".into(), Tag::Compound(state));
                c.insert("custom_particle".into(), Tag::Compound(p));
            }
        } else {
            if !effects.is_empty() {
                c.insert("Effects".into(), list(TagType::Compound, effects));
            }
            if let Some(rgb) = self.color {
                c.insert("Color".into(), Tag::Int(rgb_int(rgb)));
            }
            if !self.particles {
                c.insert("Particle".into(), Tag::String("block minecraft:air".into()));
            }
        }
        c
    }

    /// The color its particles take: the effects' colors mixed (the game's way, by level), else
    /// the color of water in a bottle.
    pub fn color(&self) -> [u8; 3] {
        self.color.unwrap_or_else(|| mixed_color(self.effects.iter().map(|e| (e.id.as_str(), e.level))))
    }

    /// A cloud as the world has it saved (either layout), to show and change its settings.
    pub fn from_entity(e: &Tag) -> Option<Cloud> {
        if !is_cloud(e) {
            return None;
        }
        let radius = e.get("Radius").and_then(Tag::as_f64).unwrap_or(3.0) as f32;
        let ticks = e.get("Duration").and_then(Tag::as_i64).unwrap_or(600);
        let list = e.get("potion_contents").and_then(|c| c.get("custom_effects")).or_else(|| e.get("Effects")).and_then(Tag::as_list);
        let effects = list
            .map(|l| {
                l.iter()
                    .filter_map(|t| {
                        let id = match t.get("id").or_else(|| t.get("Id"))? {
                            Tag::String(s) => s.trim_start_matches("minecraft:").to_string(),
                            n => crate::effects::java_effect_name(n.as_i64()?)?.to_string(),
                        };
                        let amp = t.get("amplifier").or_else(|| t.get("Amplifier")).and_then(Tag::as_i64).unwrap_or(0);
                        let dur = t.get("duration").or_else(|| t.get("Duration")).and_then(Tag::as_i64).unwrap_or(600);
                        Some(CloudEffect { id, level: amp + 1, seconds: if dur < 0 { 30 } else { (dur / 20).max(1) }, forever: dur < 0 })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let shrinks = e.get("RadiusPerTick").and_then(Tag::as_f64).is_some_and(|v| v < 0.0) || e.get("RadiusOnUse").and_then(Tag::as_f64).is_some_and(|v| v < 0.0);
        // The color it was given, if any (the effects' own mix otherwise).
        let custom = e.get("potion_contents").and_then(|c| c.get("custom_color")).or_else(|| e.get("Color")).and_then(Tag::as_i64).map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        let forever = ticks >= 1_000_000_000;
        Some(Cloud { radius, lasts: if forever { 30 } else { (ticks / 20).max(1) }, forever, effects, color: custom, particles: !particles_hidden(e), shrinks })
    }
}

/// A color as the whole number the game keeps.
fn rgb_int(rgb: [u8; 3]) -> i32 {
    ((rgb[0] as i32) << 16) | ((rgb[1] as i32) << 8) | rgb[2] as i32
}

/// Whether an entity is a lingering potion cloud.
pub fn is_cloud(e: &Tag) -> bool {
    e.get("id").and_then(Tag::as_str) == Some("minecraft:area_effect_cloud")
}

/// The mix of some effects' colors, each counted as many times as its level (as the game does).
pub fn mixed_color<'a>(effects: impl Iterator<Item = (&'a str, i64)>) -> [u8; 3] {
    let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
    for (id, level) in effects {
        let c = effect_color(id);
        let w = level.max(1) as f32;
        for k in 0..3 {
            sum[k] += c[k] as f32 * w;
        }
        n += w;
    }
    if n == 0.0 {
        return [0x38, 0x5D, 0xC6];
    }
    [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
}

/// An effect's color (the potion liquid's), as near as the game's own.
pub fn effect_color(id: &str) -> [u8; 3] {
    let hex: u32 = match id.trim_start_matches("minecraft:") {
        "speed" => 0x33EBFF,
        "slowness" => 0x8BAFE0,
        "haste" => 0xD9C043,
        "mining_fatigue" => 0x4A4217,
        "strength" => 0x932423,
        "instant_health" => 0xF82423,
        "instant_damage" => 0x430A09,
        "jump_boost" => 0xFDFF84,
        "nausea" => 0x551D4A,
        "regeneration" => 0xCD5CAB,
        "resistance" => 0x99453A,
        "fire_resistance" => 0xE49A3A,
        "water_breathing" => 0x2E5299,
        "invisibility" => 0x7F8392,
        "blindness" => 0x1F1F23,
        "night_vision" => 0x1F1FA1,
        "hunger" => 0x587653,
        "weakness" => 0x484D48,
        "poison" | "fatal_poison" => 0x4E9331,
        "wither" => 0x352A27,
        "health_boost" => 0xF87D23,
        "absorption" => 0x2552A5,
        "saturation" => 0xF82423,
        "glowing" => 0x94A061,
        "levitation" => 0xCEFFFF,
        "luck" => 0x339900,
        "unluck" => 0xC0A44D,
        "slow_falling" => 0xF3CFB9,
        "conduit_power" => 0x1DC2D1,
        "dolphins_grace" => 0x88A3BE,
        "bad_omen" => 0x0B6138,
        "hero_of_the_village" => 0x44FF44,
        "darkness" => 0x292721,
        "trial_omen" => 0x16A6A6,
        "raid_omen" => 0xDE4058,
        "wind_charged" => 0xBDC9FF,
        "weaving" => 0x78695A,
        "oozing" => 0x99FFA3,
        "infested" => 0x8C9B8C,
        _ => 0x385DC6,
    };
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

/// Whether a cloud entity's particles are hidden (an air block's), in either layout.
pub fn particles_hidden(e: &Tag) -> bool {
    let air = |s: &str| s.contains("air");
    if let Some(p) = e.get("custom_particle") {
        let is_block = p.get("type").and_then(Tag::as_str).is_some_and(|t| t.ends_with("block"));
        let state = p.get("block_state");
        let name = state.and_then(|s| s.get("Name")).and_then(Tag::as_str).or_else(|| state.and_then(Tag::as_str));
        return is_block && name.is_some_and(air);
    }
    e.get("Particle").and_then(Tag::as_str).is_some_and(|s| s.starts_with("block") && air(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_list(c: &Compound, keys: &[&str]) -> Vec<Compound> {
        let mut here = c;
        for k in &keys[..keys.len() - 1] {
            here = here.get(*k).and_then(Tag::as_compound).expect("path");
        }
        here.get(keys[keys.len() - 1]).and_then(Tag::as_list).map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect()).unwrap_or_default()
    }

    /// A cloud is written in the layout of the world's version, with what the user chose.
    #[test]
    fn a_cloud_is_written_for_each_version() {
        let mut cloud = Cloud { radius: 4.0, lasts: 60, effects: vec![CloudEffect { id: "speed".into(), level: 3, seconds: 45, forever: false }], particles: true, shrinks: false, ..Default::default() };
        // 1.21: in potion_contents, with named effects.
        let new = cloud.entity([1.5, 64.0, 2.5], 3953);
        assert_eq!(new.get("id").and_then(Tag::as_str), Some("minecraft:area_effect_cloud"));
        assert_eq!(new.get("Duration").and_then(Tag::as_i64), Some(1200));
        assert_eq!(new.get("Radius").and_then(Tag::as_f64), Some(4.0));
        assert_eq!(new.get("RadiusPerTick").and_then(Tag::as_f64), Some(0.0), "does not shrink");
        let e = &tick_list(&new, &["potion_contents", "custom_effects"])[0];
        assert_eq!(e.get("id").and_then(Tag::as_str), Some("minecraft:speed"));
        assert_eq!(e.get("amplifier").and_then(Tag::as_i64), Some(2), "level 3 is amplifier 2");
        assert_eq!(e.get("duration").and_then(Tag::as_i64), Some(900));
        assert!(new.get("custom_particle").is_none(), "particles show");
        // 1.20.4: Effects, still named.
        let mid = cloud.entity([0.0; 3], 3700);
        assert_eq!(tick_list(&mid, &["Effects"])[0].get("id").and_then(Tag::as_str), Some("minecraft:speed"));
        // 1.12: Effects, numbered (speed is 1).
        let old = cloud.entity([0.0; 3], 1343);
        assert_eq!(tick_list(&old, &["Effects"])[0].get("Id").and_then(Tag::as_i64), Some(1));
        // Particles hidden, in each layout; shrinking gives a per-tick change.
        cloud.particles = false;
        cloud.shrinks = true;
        let hidden = cloud.entity([0.0; 3], 3953);
        assert!(particles_hidden(&Tag::Compound(hidden.clone())));
        assert!(hidden.get("RadiusPerTick").and_then(Tag::as_f64).is_some_and(|v| v < 0.0));
        assert!(particles_hidden(&Tag::Compound(cloud.entity([0.0; 3], 3700))));
        assert!(!particles_hidden(&Tag::Compound(Cloud::default().entity([0.0; 3], 3953))));
    }

    /// A cloud written and read back is the same cloud, in each layout.
    #[test]
    fn a_cloud_reads_back_as_it_was_written() {
        let cloud = Cloud { radius: 5.0, lasts: 90, effects: vec![CloudEffect { id: "strength".into(), level: 2, seconds: 120, forever: false }, CloudEffect { forever: true, ..CloudEffect::new("speed") }], color: Some([10, 200, 30]), particles: false, shrinks: false, ..Default::default() };
        for dv in [3953, 3700, 1343] {
            let read = Cloud::from_entity(&Tag::Compound(cloud.entity([0.0; 3], dv))).expect("a cloud");
            assert_eq!(read, cloud, "data version {dv}");
        }
        // A cloud that never goes, with an effect that never ends.
        let ever = Cloud { forever: true, ..cloud.clone() };
        for dv in [3953, 1343] {
            let e = ever.entity([0.0; 3], dv);
            assert_eq!(e.get("Duration").and_then(Tag::as_i64), Some(i64::from(i32::MAX)));
            assert_eq!(Cloud::from_entity(&Tag::Compound(e)).expect("a cloud"), Cloud { lasts: 30, ..ever.clone() }, "data version {dv}");
        }
        assert!(Cloud::from_entity(&Tag::Compound(Compound::new())).is_none(), "not a cloud");
    }

    /// Each cloud has its own identity, and the colors mix by level.
    #[test]
    fn clouds_have_their_own_identity_and_a_mixed_color() {
        let a = Cloud::default().entity([0.0; 3], 3953);
        let b = Cloud::default().entity([0.0; 3], 3953);
        assert_ne!(a.get("UUID"), b.get("UUID"));
        assert_eq!(effect_color("poison"), [0x4E, 0x93, 0x31]);
        assert_eq!(mixed_color(std::iter::empty()), [0x38, 0x5D, 0xC6], "water's color with none");
        let mix = mixed_color([("speed", 1), ("slowness", 1)].into_iter());
        assert!(mix[0] > 0x33 && mix[0] < 0x8B, "between the two");
    }
}
