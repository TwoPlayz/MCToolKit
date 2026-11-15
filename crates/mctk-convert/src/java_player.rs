//! Player data between Java versions: each player's NBT file, advancements and statistics, and the
//! host player of a single-player world.
//!
//! Layouts:
//! * classic (1.20.x): `playerdata/<uuid>.dat`, `advancements/<uuid>.json`, `stats/<uuid>.json`,
//!   and the host player is also embedded in `level.dat` (`Data.Player`).
//! * modern (26.x): `players/data/<uuid>.dat`, `players/advancements/<uuid>.json`,
//!   `players/stats/<uuid>.json`; `level.dat` only names the host (`singleplayer_uuid`).
//!
//! Going up, a player file is moved as it is, keeping its `DataVersion`: the game upgrades it with
//! its own data fixer when the player joins, which no converter can do better. Going down there is
//! no such help, so the file is converted here: items (in `Inventory` and `EnderItems`), armor and
//! the offhand (`equipment` in modern, inventory slots 100 – 103 and -106 in classic), the spawn
//! point (`respawn` against `SpawnX/Y/Z`...), attributes, effects and the rest of what mobs share.

use std::path::Path;

use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};
use mctk_registry::Registry;

use crate::Error;
use crate::java_entities::{Form, convert_stack_list, entity_fields};
use crate::java_items::{Notes, item_down, s};

/// One player and the files that belong to the same UUID.
#[derive(Debug, Clone)]
pub struct PlayerFile {
    /// Dashed UUID, the file name.
    pub uuid: String,
    pub nbt: Compound,
    pub advancements: Option<Vec<u8>>,
    pub stats: Option<Vec<u8>>,
}

/// `[I;a,b,c,d]` as the dashed UUID text used in file names.
pub fn uuid_text(u: [i32; 4]) -> String {
    let (a, b, c, d) = (u[0] as u32, u[1] as u32, u[2] as u32, u[3] as u32);
    format!("{a:08x}-{:04x}-{:04x}-{:04x}-{:04x}{d:08x}", b >> 16, b & 0xffff, c >> 16, c & 0xffff)
}

fn read_gz(path: &Path) -> Result<Compound, Error> {
    let (_, root) = mctk_nbt::read_file(path, Flavor::Java)?;
    root.tag.as_compound().cloned().ok_or_else(|| Error::Invalid(format!("{} is not a compound", path.display())))
}

fn write_gz(path: &Path, c: &Compound) -> Result<(), Error> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let raw = mctk_nbt::to_bytes(&NamedTag { name: String::new(), tag: Tag::Compound(c.clone()) }, Flavor::Java)?;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw)?;
    std::fs::write(path, e.finish()?)?;
    Ok(())
}

/// Reads every player of a world folder in either layout. `host` is the single-player host
/// (from `level.dat`), whose embedded copy (classic) is used when it has no file of its own.
pub fn read_players(world: &Path, modern: bool, host: Option<(&[i32; 4], &Compound)>, level_dv: i32) -> Result<Vec<PlayerFile>, Error> {
    let (data, adv, stats) = if modern {
        (world.join("players/data"), world.join("players/advancements"), world.join("players/stats"))
    } else {
        (world.join("playerdata"), world.join("advancements"), world.join("stats"))
    };
    let mut out: Vec<PlayerFile> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&data) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("dat") {
                continue;
            }
            let Some(uuid) = path.file_stem().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            let mut nbt = read_gz(&path)?;
            nbt.entry("DataVersion".into()).or_insert(Tag::Int(level_dv));
            out.push(PlayerFile {
                advancements: std::fs::read(adv.join(format!("{uuid}.json"))).ok(),
                stats: std::fs::read(stats.join(format!("{uuid}.json"))).ok(),
                uuid,
                nbt,
            });
        }
    }
    // The classic host lives in level.dat; it counts when no file was written for it.
    if let Some((uuid, player)) = host {
        let text = uuid_text(*uuid);
        if !out.iter().any(|p| p.uuid == text) {
            let mut nbt = player.clone();
            nbt.entry("DataVersion".into()).or_insert(Tag::Int(level_dv));
            out.push(PlayerFile {
                advancements: std::fs::read(adv.join(format!("{text}.json"))).ok(),
                stats: std::fs::read(stats.join(format!("{text}.json"))).ok(),
                uuid: text,
                nbt,
            });
        }
    }
    out.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    Ok(out)
}

/// Converts a modern player to the classic form, in place.
pub fn player_to_classic(p: &mut Compound, target: &Registry, notes: &mut Notes) {
    // Armor and the offhand go back to inventory slots; take them out before the generic pass,
    // which would turn `equipment` into the mob fields `ArmorItems` and `HandItems`.
    let equipment = p.shift_remove("equipment");
    entity_fields(p, Form::Classic, target, notes);
    if let Some(t) = p.get("EnderItems").cloned() {
        p.insert("EnderItems".into(), convert_stack_list(&t, Form::Classic, notes));
    }
    if let Some(Tag::Compound(eq)) = equipment {
        let mut inventory: Vec<Tag> = match p.shift_remove("Inventory") {
            Some(Tag::List(l)) => l.iter().cloned().collect(),
            _ => Vec::new(),
        };
        for (slot, number) in [("feet", 100i8), ("legs", 101), ("chest", 102), ("head", 103), ("offhand", -106)] {
            if let Some(Tag::Compound(item)) = eq.get(slot) {
                let mut stack = Compound::from([("Slot".to_string(), Tag::Byte(number))]);
                stack.append(&mut item_down(item, notes));
                inventory.push(Tag::Compound(stack));
            }
        }
        for other in eq.keys().filter(|k| !["feet", "legs", "chest", "head", "offhand"].contains(&k.as_str())) {
            notes.add(format!("player equipment slot {other} does not exist before 1.20.5"));
        }
        p.insert("Inventory".into(), Tag::List(List::from_tags(TagType::Compound, inventory).expect("compounds")));
    }
    // The spawn point: one `respawn` compound against six loose keys.
    if let Some(Tag::Compound(r)) = p.shift_remove("respawn") {
        if let Some(Tag::IntArray(pos)) = r.get("pos") {
            if pos.len() == 3 {
                for (k, v) in ["SpawnX", "SpawnY", "SpawnZ"].iter().zip(pos) {
                    p.insert((*k).into(), Tag::Int(*v));
                }
            }
        }
        if let Some(d) = r.get("dimension") {
            p.insert("SpawnDimension".into(), d.clone());
        }
        let yaw = r.get("yaw").or_else(|| r.get("angle")).and_then(Tag::as_f64).unwrap_or(0.0);
        p.insert("SpawnAngle".into(), Tag::Float(yaw as f32));
        p.insert("SpawnForced".into(), Tag::Byte(r.get("forced").and_then(Tag::as_i64).unwrap_or(0) as i8));
    }
    for key in ["entered_nether_pos", "raid_omen_position", "ender_pearls", "ender_pearl_dimension", "spawn_extra_particles_on_fall", "post_effects", "current_impulse_context_reset_grace_time"] {
        p.shift_remove(key);
    }
    for key in ["ShoulderEntityLeft", "ShoulderEntityRight"] {
        if let Some(Tag::Compound(e)) = p.get_mut(key) {
            entity_fields(e, Form::Classic, target, notes);
        }
    }
    let _ = s;
}

/// Writes the players into `dst` in the layout of the target and returns the host's classic copy
/// (for `level.dat`) when there is one. `from` is the source layout.
pub fn write_players(
    dst: &Path,
    players: &[PlayerFile],
    to_modern: bool,
    to_form: Form,
    target: &Registry,
    host: Option<&[i32; 4]>,
    notes: &mut Notes,
) -> Result<Option<Compound>, Error> {
    let (data, adv, stats) = if to_modern {
        (dst.join("players/data"), dst.join("players/advancements"), dst.join("players/stats"))
    } else {
        (dst.join("playerdata"), dst.join("advancements"), dst.join("stats"))
    };
    let host_text = host.map(|u| uuid_text(*u));
    let mut embedded = None;
    for p in players {
        let mut nbt = p.nbt.clone();
        let source_dv = nbt.get("DataVersion").and_then(Tag::as_i64).map(|v| v as i32);
        let source_modern = source_dv.and_then(Form::of) == Some(Form::Modern);
        // 1.20.5 to 1.21.4: components already, but JSON text and `HandItems` still.
        let source_era = source_dv.and_then(Form::source) == Some((Form::Modern, true));
        if to_form == Form::Classic && (source_modern || source_era) {
            if source_era {
                crate::java_entities::era_up_entity(&mut nbt, notes);
            }
            player_to_classic(&mut nbt, target, notes);
            nbt.insert("DataVersion".into(), Tag::Int(target.data_version.unwrap_or(3465)));
        }
        // 1.8 to 1.12 take numeric items and short names; players of newer data get them here.
        let target_dv = target.data_version.unwrap_or(3465);
        if crate::java_numeric::is_numeric(target_dv) {
            let source_dv = p.nbt.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0) as i32;
            if source_dv > crate::java_numeric::LAST_NUMERIC {
                crate::java_numeric::player(&mut nbt, crate::java_numeric::era(target_dv), notes);
            }
            if target_dv > 0 {
                nbt.insert("DataVersion".into(), Tag::Int(target_dv));
            } else {
                nbt.shift_remove("DataVersion");
            }
        }
        write_gz(&data.join(format!("{}.dat", p.uuid)), &nbt)?;
        if !to_modern && host_text.as_deref() == Some(p.uuid.as_str()) {
            let mut c = nbt.clone();
            c.shift_remove("DataVersion");
            embedded = Some(c);
        }
        for (bytes, dir, ext) in [(&p.advancements, &adv, "json"), (&p.stats, &stats, "json")] {
            if let Some(b) = bytes {
                std::fs::create_dir_all(dir)?;
                std::fs::write(dir.join(format!("{}.{ext}", p.uuid)), b)?;
            }
        }
    }
    Ok(embedded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_registry::Edition;

    fn c(fields: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    fn list(ty: TagType, items: Vec<Tag>) -> Tag {
        Tag::List(List::from_tags(ty, items).unwrap())
    }
    fn modern_item(id: &str, count: i32, extra: Vec<(&str, Tag)>) -> Tag {
        let mut f = vec![("id", s(id)), ("count", Tag::Int(count))];
        f.extend(extra);
        c(f)
    }

    fn registry() -> Registry {
        let mut r = Registry::new(Edition::Java, "1.20.1");
        r.data_version = Some(3465);
        r
    }

    /// A 26.x player as the game saves it: armor and offhand in `equipment`, the spawn point as
    /// `respawn`, components on items, attributes and effects with their new names.
    fn modern_player() -> Compound {
        let sword = modern_item(
            "minecraft:diamond_sword",
            1,
            vec![("Slot", Tag::Byte(0)), ("components", c(vec![("minecraft:damage", Tag::Int(7)), ("minecraft:enchantments", c(vec![("minecraft:sharpness", Tag::Int(4))]))]))],
        );
        let mut p = Compound::new();
        p.insert("DataVersion".into(), Tag::Int(5023));
        p.insert("Health".into(), Tag::Float(14.0));
        p.insert("foodLevel".into(), Tag::Int(11));
        p.insert("XpLevel".into(), Tag::Int(23));
        p.insert("fall_distance".into(), Tag::Double(0.0));
        p.insert("Inventory".into(), list(TagType::Compound, vec![sword, modern_item("minecraft:cooked_beef", 37, vec![("Slot", Tag::Byte(1))])]));
        p.insert("EnderItems".into(), list(TagType::Compound, vec![modern_item("minecraft:diamond", 9, vec![("Slot", Tag::Byte(0))])]));
        p.insert(
            "equipment".into(),
            c(vec![
                ("feet", modern_item("minecraft:iron_boots", 1, vec![("components", c(vec![("minecraft:damage", Tag::Int(20))]))])),
                ("chest", modern_item("minecraft:diamond_chestplate", 1, vec![])),
                ("head", modern_item("minecraft:turtle_helmet", 1, vec![])),
                ("offhand", modern_item("minecraft:shield", 1, vec![])),
            ]),
        );
        p.insert("respawn".into(), c(vec![("pos", Tag::IntArray(vec![-48, 121, -16])), ("dimension", s("minecraft:overworld")), ("yaw", Tag::Float(90.0)), ("pitch", Tag::Float(0.0)), ("forced", Tag::Byte(1))]));
        p.insert("attributes".into(), list(TagType::Compound, vec![c(vec![("id", s("minecraft:movement_speed")), ("base", Tag::Double(0.1))])]));
        p.insert("active_effects".into(), list(TagType::Compound, vec![c(vec![("id", s("minecraft:speed")), ("amplifier", Tag::Byte(1)), ("duration", Tag::Int(4000))])]));
        p
    }

    #[test]
    fn downgrading_a_player_restores_every_classic_field() {
        let mut p = modern_player();
        let mut notes = Notes::default();
        player_to_classic(&mut p, &registry(), &mut notes);
        assert!(notes.0.is_empty(), "{notes:?}");

        // Stats survive untouched; the generic mob fields come back in their old form.
        assert_eq!(p.get("Health"), Some(&Tag::Float(14.0)));
        assert_eq!(p.get("foodLevel"), Some(&Tag::Int(11)));
        assert_eq!(p.get("FallDistance"), Some(&Tag::Float(0.0)));
        assert_eq!(p.get("Attributes").unwrap().as_list().unwrap().get(0).unwrap().get("Name"), Some(&s("minecraft:generic.movement_speed")));
        assert_eq!(p.get("ActiveEffects").unwrap().as_list().unwrap().get(0).unwrap().get("Id"), Some(&Tag::Byte(1)));

        // Items are classic stacks, and armor and the offhand are inventory slots again.
        let inv = p.get("Inventory").and_then(Tag::as_list).unwrap();
        let by_slot = |n: i64| inv.iter().find(|t| t.get("Slot").and_then(Tag::as_i64) == Some(n)).cloned().unwrap_or_else(|| panic!("no slot {n}"));
        let sword = by_slot(0);
        assert_eq!(sword.get("Count"), Some(&Tag::Byte(1)));
        assert_eq!(sword.get("tag").unwrap().get("Damage"), Some(&Tag::Int(7)));
        assert_eq!(by_slot(1).get("Count"), Some(&Tag::Byte(37)));
        assert_eq!(by_slot(100).get("id"), Some(&s("minecraft:iron_boots")));
        assert_eq!(by_slot(100).get("tag").unwrap().get("Damage"), Some(&Tag::Int(20)));
        assert_eq!(by_slot(102).get("id"), Some(&s("minecraft:diamond_chestplate")));
        assert_eq!(by_slot(103).get("id"), Some(&s("minecraft:turtle_helmet")));
        assert_eq!(by_slot(-106).get("id"), Some(&s("minecraft:shield")));
        assert!(inv.iter().all(|t| t.get("Slot").and_then(Tag::as_i64) != Some(101)), "no leggings were worn");
        assert!(p.get("equipment").is_none());

        // The ender chest holds classic stacks too.
        assert_eq!(p.get("EnderItems").unwrap().as_list().unwrap().get(0).unwrap().get("Count"), Some(&Tag::Byte(9)));

        // One respawn compound becomes the six spawn keys.
        assert_eq!((p.get("SpawnX"), p.get("SpawnY"), p.get("SpawnZ")), (Some(&Tag::Int(-48)), Some(&Tag::Int(121)), Some(&Tag::Int(-16))));
        assert_eq!(p.get("SpawnAngle"), Some(&Tag::Float(90.0)));
        assert_eq!(p.get("SpawnForced"), Some(&Tag::Byte(1)));
        assert_eq!(p.get("SpawnDimension"), Some(&s("minecraft:overworld")));
        assert!(p.get("respawn").is_none());
    }

    #[test]
    fn uuid_text_is_the_dashed_form_of_the_file_name() {
        // The host of the sample world.
        assert_eq!(uuid_text([-1996495974, 974998821, -1499058141, 1483921170]), "88ffe39a-3a1d-4d25-a6a6-30235872d712");
    }

    #[test]
    fn players_round_trip_through_the_files_and_the_host_is_embedded() {
        let dir = std::env::temp_dir().join(format!("mctk-player-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut p = modern_player();
        p.insert("UUID".into(), Tag::IntArray(vec![-1996495974, 974998821, -1499058141, 1483921170]));
        let player = PlayerFile {
            uuid: "88ffe39a-3a1d-4d25-a6a6-30235872d712".into(),
            nbt: p,
            advancements: Some(br#"{"a":1}"#.to_vec()),
            stats: Some(br#"{"stats":{}}"#.to_vec()),
        };
        let host = [-1996495974, 974998821, -1499058141, 1483921170];
        let mut notes = Notes::default();
        let embedded = write_players(&dir, &[player], false, Form::Classic, &registry(), Some(&host), &mut notes).unwrap();
        assert!(embedded.as_ref().is_some_and(|e| e.contains_key("Inventory") && !e.contains_key("DataVersion")));
        for f in ["playerdata/88ffe39a-3a1d-4d25-a6a6-30235872d712.dat", "advancements/88ffe39a-3a1d-4d25-a6a6-30235872d712.json", "stats/88ffe39a-3a1d-4d25-a6a6-30235872d712.json"] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        // Read back as a classic world; the file's DataVersion is the target's.
        let back = read_players(&dir, false, None, 3465).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].nbt.get("DataVersion"), Some(&Tag::Int(3465)));
        assert!(back[0].advancements.is_some() && back[0].stats.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
