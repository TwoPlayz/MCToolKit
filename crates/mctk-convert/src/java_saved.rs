//! Saved-data files that sit beside the chunks, between the classic layout (1.20.x) and the modern
//! one (26.x). The placement was taken from a real 26.3 server migrating a real 1.20.1 world:
//!
//! | classic                                  | modern                                              |
//! |------------------------------------------|-----------------------------------------------------|
//! | `data/scoreboard.dat`                    | `data/minecraft/scoreboard.dat`                     |
//! | `data/command_storage_<ns>.dat`          | `data/<ns>/command_storage.dat`                     |
//! | `data/map_<n>.dat`                       | `data/minecraft/maps/<n>.dat`                       |
//! | `data/idcounts.dat`                      | `data/minecraft/maps/last_id.dat`                   |
//! | `data/raids.dat` (+ `DIM-1`, `DIM1/raids_end`) | `dimensions/minecraft/<dim>/data/minecraft/raids.dat` |
//!
//! Going up the files are copied as they are, keeping their `DataVersion`; the game upgrades each
//! one when it loads it (the migration itself left maps and command storage byte for byte). Going
//! down nothing can do that for us, so the scoreboard, maps and raids are converted here.

use std::path::Path;

use mctk_nbt::{Compound, Tag};

use crate::Error;
use crate::java_items::Notes;
use crate::java_level::{read_data, write_data_file};
use crate::java_text::{nbt_to_json};

/// Display slots by their classic key; team sidebars follow the 16 chat colours.
const COLOURS: [&str; 16] = [
    "black", "dark_blue", "dark_green", "dark_aqua", "dark_red", "dark_purple", "gold", "gray",
    "dark_gray", "blue", "green", "aqua", "red", "light_purple", "yellow", "white",
];

/// The (classic, modern) display slot names.
fn slot_names() -> Vec<(String, String)> {
    let mut v = vec![
        ("slot_0".to_string(), "list".to_string()),
        ("slot_1".to_string(), "sidebar".to_string()),
        ("slot_2".to_string(), "below_name".to_string()),
    ];
    for (i, c) in COLOURS.iter().enumerate() {
        v.push((format!("slot_{}", i + 3), format!("sidebar.team.{c}")));
    }
    v
}

fn text_to_json(c: &mut Compound, key: &str) {
    if let Some(t) = c.get(key).cloned() {
        c.insert(key.to_string(), Tag::String(nbt_to_json(&t)));
    }
}

/// A modern scoreboard as the classic form: JSON names, `slot_N` display slots, and the fields
/// the classic game requires.
pub fn scoreboard_to_classic(data: &Compound) -> Compound {
    let mut out = data.clone();
    if let Some(Tag::List(objs)) = out.get_mut("Objectives") {
        for o in objs.iter_mut().filter_map(Tag::as_compound_mut) {
            text_to_json(o, "DisplayName");
            o.entry("CriteriaName".into()).or_insert_with(|| Tag::String("dummy".into()));
            o.entry("RenderType".into()).or_insert_with(|| Tag::String("integer".into()));
        }
    }
    if let Some(Tag::Compound(slots)) = out.get("DisplaySlots").cloned() {
        let names = slot_names();
        let renamed: Compound = slots
            .into_iter()
            .map(|(k, v)| (names.iter().find(|(_, m)| *m == k).map_or(k.clone(), |(c, _)| c.clone()), v))
            .collect();
        out.insert("DisplaySlots".into(), Tag::Compound(renamed));
    }
    if let Some(Tag::List(teams)) = out.get_mut("Teams") {
        for t in teams.iter_mut().filter_map(Tag::as_compound_mut) {
            for k in ["DisplayName", "MemberNamePrefix", "MemberNameSuffix"] {
                text_to_json(t, k);
            }
        }
    }
    out.entry("Teams".into()).or_insert_with(|| Tag::List(mctk_nbt::List::new(mctk_nbt::TagType::Compound)));
    out.entry("Objectives".into()).or_insert_with(|| Tag::List(mctk_nbt::List::new(mctk_nbt::TagType::Compound)));
    out.entry("PlayerScores".into()).or_insert_with(|| Tag::List(mctk_nbt::List::new(mctk_nbt::TagType::Compound)));
    out
}

/// A modern map as the classic form: banner names back to JSON text.
pub fn map_to_classic(data: &Compound) -> Compound {
    let mut out = data.clone();
    if let Some(Tag::List(banners)) = out.get_mut("banners") {
        for b in banners.iter_mut().filter_map(Tag::as_compound_mut) {
            text_to_json(b, "Name");
        }
    }
    out
}

fn copy_raw(from: &Path, to: &Path) -> Result<u64, Error> {
    if !from.is_file() {
        return Ok(0);
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(from, to)?;
    Ok(1)
}

/// `(classic raids file relative to the world, modern dimension folder name)`
const RAIDS: [(&str, &str, &str); 3] = [
    ("data/raids.dat", "overworld", "minecraft:overworld"),
    ("DIM-1/data/raids.dat", "the_nether", "minecraft:the_nether"),
    ("DIM1/data/raids_end.dat", "the_end", "minecraft:the_end"),
];

/// Classic world files into the modern layout, unchanged. Returns how many files moved.
pub fn up(src: &Path, dst: &Path, included: &[&str]) -> Result<u64, Error> {
    let mut n = copy_raw(&src.join("data/scoreboard.dat"), &dst.join("data/minecraft/scoreboard.dat"))?;
    n += copy_raw(&src.join("data/idcounts.dat"), &dst.join("data/minecraft/maps/last_id.dat"))?;
    if let Ok(entries) = std::fs::read_dir(src.join("data")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(ns) = name.strip_prefix("command_storage_").and_then(|n| n.strip_suffix(".dat")) {
                n += copy_raw(&e.path(), &dst.join("data").join(ns).join("command_storage.dat"))?;
            } else if let Some(id) = name.strip_prefix("map_").and_then(|n| n.strip_suffix(".dat")).filter(|i| i.parse::<u64>().is_ok()) {
                n += copy_raw(&e.path(), &dst.join(format!("data/minecraft/maps/{id}.dat")))?;
            }
        }
    }
    for (classic, dim, id) in RAIDS {
        if included.contains(&id) {
            n += copy_raw(
                &src.join(classic),
                &dst.join(format!("dimensions/minecraft/{dim}/data/minecraft/raids.dat")),
            )?;
        }
    }
    Ok(n)
}

/// Modern world files into the classic layout, converted. Returns how many files were written.
pub fn down(src: &Path, dst: &Path, included: &[&str], data_version: i32, notes: &mut Notes) -> Result<u64, Error> {
    let mut n = 0;
    // From 1.21.5 on the files are in the modern form already; only their place is the old one.
    let modern_form = data_version >= 4325;
    if let Some(sb) = read_data(&src.join("data/minecraft/scoreboard.dat"), "data")? {
        let sb = if modern_form { sb } else { scoreboard_to_classic(&sb) };
        write_data_file(&dst.join("data/scoreboard.dat"), sb, data_version)?;
        n += 1;
    }
    if let Some(c) = read_data(&src.join("data/minecraft/maps/last_id.dat"), "data")? {
        write_data_file(&dst.join("data/idcounts.dat"), c, data_version)?;
        n += 1;
    }
    if let Ok(entries) = std::fs::read_dir(src.join("data/minecraft/maps")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".dat").filter(|i| i.parse::<u64>().is_ok()) {
                if let Some(map) = read_data(&e.path(), "data")? {
                    let map = if modern_form { map } else { map_to_classic(&map) };
                    write_data_file(&dst.join(format!("data/map_{id}.dat")), map, data_version)?;
                    n += 1;
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(src.join("data")) {
        for e in entries.flatten() {
            let ns = e.file_name().to_string_lossy().into_owned();
            let file = e.path().join("command_storage.dat");
            if let Some(c) = read_data(&file, "data")? {
                write_data_file(&dst.join(format!("data/command_storage_{ns}.dat")), c, data_version)?;
                n += 1;
            }
        }
    }
    for (classic, dim, id) in RAIDS {
        if !included.contains(&id) {
            continue;
        }
        let file = src.join(format!("dimensions/minecraft/{dim}/data/minecraft/raids.dat"));
        let Some(r) = read_data(&file, "data")? else { continue };
        if modern_form {
            write_data_file(&dst.join(classic), r, data_version)?;
            n += 1;
            continue;
        }
        // A file moved up unchanged still has the classic keys until the game rewrites it.
        if matches!(r.get("raids").or_else(|| r.get("Raids")), Some(Tag::List(l)) if !l.is_empty()) {
            notes.add("active raids are not converted (their format changed)");
        }
        let num = |modern: &str, classic: &str, default: i64| {
            r.get(modern).or_else(|| r.get(classic)).and_then(Tag::as_i64).unwrap_or(default) as i32
        };
        let converted = Compound::from([
            ("Raids".to_string(), Tag::List(mctk_nbt::List::new(mctk_nbt::TagType::Compound))),
            ("NextAvailableID".to_string(), Tag::Int(num("next_id", "NextAvailableID", 1))),
            ("Tick".to_string(), Tag::Int(num("tick", "Tick", 0))),
        ]);
        write_data_file(&dst.join(classic), converted, data_version)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_nbt::{List, TagType};

    fn c(fields: Vec<(&str, Tag)>) -> Compound {
        fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }
    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }
    fn list(items: Vec<Tag>) -> Tag {
        Tag::List(List::from_tags(TagType::Compound, items).unwrap())
    }

    /// What the real 26.3 server wrote for the scoreboard of the test world (see the module docs).
    fn modern_scoreboard() -> Compound {
        c(vec![
            ("PlayerScores", list(vec![Tag::Compound(c(vec![("Objective", s("coins")), ("Locked", Tag::Byte(1)), ("Score", Tag::Int(42)), ("Name", s("Steve"))]))])),
            ("DisplaySlots", Tag::Compound(c(vec![("sidebar", s("coins")), ("sidebar.team.red", s("kills"))]))),
            (
                "Objectives",
                list(vec![
                    Tag::Compound(c(vec![("DisplayName", s("Coins")), ("Name", s("coins"))])),
                    Tag::Compound(c(vec![("CriteriaName", s("minecraft.killed:minecraft.zombie")), ("DisplayName", Tag::Compound(c(vec![("color", s("red")), ("text", s("Zombie Kills"))]))), ("Name", s("kills"))])),
                ]),
            ),
        ])
    }

    #[test]
    fn scoreboard_goes_back_to_the_classic_form() {
        let out = scoreboard_to_classic(&modern_scoreboard());
        let slots = out.get("DisplaySlots").and_then(Tag::as_compound).unwrap();
        assert_eq!(slots.get("slot_1"), Some(&s("coins")));
        assert_eq!(slots.get("slot_15"), Some(&s("kills")), "red team sidebar is slot 3 + index of red (12)");
        assert!(!slots.contains_key("sidebar"));
        let objs = out.get("Objectives").and_then(Tag::as_list).unwrap();
        let coins = objs.get(0).unwrap();
        // The fields the classic game needs are filled in; names are JSON text.
        assert_eq!(coins.get("CriteriaName"), Some(&s("dummy")));
        assert_eq!(coins.get("RenderType"), Some(&s("integer")));
        let name: serde_json::Value = serde_json::from_str(coins.get("DisplayName").and_then(Tag::as_str).unwrap()).unwrap();
        assert_eq!(name["text"], "Coins");
        let kills: serde_json::Value = serde_json::from_str(objs.get(1).unwrap().get("DisplayName").and_then(Tag::as_str).unwrap()).unwrap();
        assert_eq!((kills["text"].as_str(), kills["color"].as_str()), (Some("Zombie Kills"), Some("red")));
        assert_eq!(out.get("PlayerScores"), modern_scoreboard().get("PlayerScores"));
        assert!(out.contains_key("Teams"));
    }

    #[test]
    fn files_move_to_the_places_the_game_uses_and_come_back() {
        let tmp = std::env::temp_dir().join(format!("mctk-saved-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (classic, modern, back) = (tmp.join("classic"), tmp.join("modern"), tmp.join("back"));
        std::fs::create_dir_all(classic.join("data")).unwrap();
        std::fs::create_dir_all(classic.join("DIM-1/data")).unwrap();
        for (path, data) in [
            ("data/scoreboard.dat", modern_scoreboard()),
            ("data/command_storage_mctk.dat", c(vec![("contents", Tag::Compound(c(vec![("a", Tag::Int(1))])))])),
            ("data/map_0.dat", c(vec![("scale", Tag::Byte(1)), ("banners", list(vec![Tag::Compound(c(vec![("Name", s("Home"))]))]))])),
            ("data/idcounts.dat", c(vec![("map", Tag::Int(0))])),
            ("data/raids.dat", c(vec![("NextAvailableID", Tag::Int(1)), ("Tick", Tag::Int(99)), ("Raids", Tag::List(List::new(TagType::Compound)))])),
            ("DIM-1/data/raids.dat", c(vec![("NextAvailableID", Tag::Int(1)), ("Tick", Tag::Int(7)), ("Raids", Tag::List(List::new(TagType::Compound)))])),
        ] {
            write_data_file(&classic.join(path), data, 3465).unwrap();
        }
        let dims = ["minecraft:overworld", "minecraft:the_nether"];

        // Up: the placement the real game produced, contents untouched.
        assert_eq!(up(&classic, &modern, &dims).unwrap(), 6);
        for p in [
            "data/minecraft/scoreboard.dat",
            "data/mctk/command_storage.dat",
            "data/minecraft/maps/0.dat",
            "data/minecraft/maps/last_id.dat",
            "dimensions/minecraft/overworld/data/minecraft/raids.dat",
            "dimensions/minecraft/the_nether/data/minecraft/raids.dat",
        ] {
            assert!(modern.join(p).is_file(), "{p}");
        }
        assert_eq!(read_data(&modern.join("data/minecraft/maps/0.dat"), "data").unwrap().unwrap().get("scale"), Some(&Tag::Byte(1)));

        // Down again: converted for the classic game, with its DataVersion.
        let mut notes = Notes::default();
        let n = down(&modern, &back, &dims, 3465, &mut notes).unwrap();
        assert_eq!(n, 6, "{notes:?}");
        assert!(back.join("data/command_storage_mctk.dat").is_file() && back.join("data/idcounts.dat").is_file());
        let map = read_data(&back.join("data/map_0.dat"), "data").unwrap().unwrap();
        let banner_name = map.get("banners").and_then(Tag::as_list).unwrap().get(0).unwrap().get("Name").and_then(Tag::as_str).unwrap().to_string();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&banner_name).unwrap()["text"], "Home");
        let raids = read_data(&back.join("DIM-1/data/raids.dat"), "data").unwrap().unwrap();
        assert_eq!(raids.get("Tick"), Some(&Tag::Int(7)));
        assert!(notes.0.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
