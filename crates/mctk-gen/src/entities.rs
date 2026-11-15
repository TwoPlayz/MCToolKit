//! Asks the Bedrock server which component groups a freshly spawned actor starts with.
//!
//! A Bedrock actor on disk records which of its type's component groups are active
//! (`definitions`, e.g. `+minecraft:rabbit`, `+adult`). The game picks them when the entity spawns.
//! `entity-plan <ids.txt> <commands.txt>` writes the server commands that summon each type twice
//! (as it spawns naturally, and as `minecraft:entity_born`, the baby), on two rows far apart;
//! `entity-read <world> <entities.tsv>` reads the actors the server saved:
//! `identifier<TAB>adult definitions<TAB>baby definitions`.

use std::collections::BTreeMap;
use std::path::Path;

use mctk_bedrock::{BedrockWorld, Key};
use mctk_nbt::{Flavor, Tag};

const COLUMNS: usize = 12;
const SPACING: usize = 8;
/// The baby row starts this far from the adult row.
const BABY_Z: i32 = 120;

fn spot(i: usize) -> (i32, i32) {
    (((i % COLUMNS) * SPACING) as i32 + 4, ((i / COLUMNS) * SPACING) as i32 + 4)
}

pub fn plan(ids: &Path, out: &Path) -> Result<(), String> {
    let ids: Vec<String> = std::fs::read_to_string(ids).map_err(|e| e.to_string())?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let rows = ids.len().div_ceil(COLUMNS) as i32 * SPACING as i32;
    let mut text = String::new();
    text.push_str("difficulty easy\ngamerule domobspawning false\ngamerule dodaylightcycle false\ngamerule domobloot false\ntime set 18000\n");
    for (name, z0) in [("adults", 0), ("babies", BABY_Z)] {
        text.push_str(&format!("tickingarea add 0 -59 {z0} 95 -59 {} {name}\n", z0 + rows));
    }
    text.push_str("#sleep 20\n");
    // Mobs vanish within seconds with no player near, so the world is copied (while the server
    // holds its saves) after every batch of summons and the copies are read.
    let mut batch = 0;
    let mut jobs: Vec<(usize, bool)> = Vec::new();
    for i in 0..ids.len() {
        jobs.push((i, false));
        jobs.push((i, true));
    }
    for (n, chunk) in jobs.chunks(std::env::var("ENT_BATCH").ok().and_then(|v| v.parse().ok()).unwrap_or(8)).enumerate() {
        for &(i, born) in chunk {
            let (x, z) = spot(i);
            if born {
                text.push_str(&format!("summon {} {x} -59 {} 0 0 minecraft:entity_born\n", ids[i], z + BABY_Z));
            } else {
                // A few types spawn as babies now and then; several are summoned.
                for j in 0..std::env::var("ENT_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1) {
                    text.push_str(&format!("summon {} {} -59 {z} 0 0 minecraft:entity_spawned\n", ids[i], x + j));
                }
            }
        }
        text.push_str(&format!("#snapshot {}{n:03}\n", std::env::var("ENT_PREFIX").unwrap_or_else(|_| "b".into())));
        batch += 1;
    }
    let _ = batch;    text.push_str("#sleep 10\n");
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("{} commands for {} types", ids.len() * 2, ids.len());
    Ok(())
}

pub fn read(snapshots: &Path, out: &Path) -> Result<(), String> {
    let mut adult: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut baby: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(snapshots).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
    let Ok(world) = BedrockWorld::open(&dir) else { continue };
    for (key, value) in &world.db.entries {
        if !matches!(Key::decode(key), Key::Actor(_)) {
            continue;
        }
        let Ok(root) = mctk_nbt::from_bytes(value, Flavor::Bedrock) else { continue };
        let Some(id) = root.tag.get("identifier").and_then(Tag::as_str) else { continue };
        let defs: Vec<String> = match root.tag.get("definitions") {
            Some(Tag::List(l)) => l.iter().filter_map(|t| t.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        let z = match root.tag.get("Pos") {
            Some(Tag::List(l)) => l.get(2).and_then(|t| t.as_f64()).unwrap_or(0.0),
            _ => 0.0,
        };
        let map = if z > f64::from(BABY_Z) - 30.0 { &mut baby } else { &mut adult };
        let _ = is_baby_defs;
        let is_baby = defs.iter().any(|d| d.to_lowercase().contains("baby"));
        let slot = map.entry(id.to_string()).or_default();
        // The adult row keeps a sample that is not a baby; the baby row keeps the first.
        if slot.is_empty() || (z <= f64::from(BABY_Z) - 30.0 && is_baby_defs(slot) && !is_baby) {
            *slot = defs;
        }
    }
    }
    let mut text = String::new();
    let mut names: Vec<&String> = adult.keys().chain(baby.keys()).collect();
    names.sort();
    names.dedup();
    for id in &names {
        let a = adult.get(*id).map(|d| d.join(",")).unwrap_or_default();
        let b = baby.get(*id).map(|d| d.join(",")).filter(|b| *b != a).unwrap_or_default();
        text.push_str(&format!("{id}\t{a}\t{b}\n"));
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("{} entity types", names.len());
    Ok(())
}
fn is_baby_defs(defs: &[String]) -> bool {
    defs.iter().any(|d| d.to_lowercase().contains("baby"))
}
