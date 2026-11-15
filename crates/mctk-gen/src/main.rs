//! Builds version tables by asking the games.
//!
//! Bedrock has no block list on disk: blocks are compiled into the executable. So the real
//! Bedrock Dedicated Server is the source of truth, and this tool drives it:
//!
//! 1. `strings <exe> <out>` lists every `minecraft:<name>` string in the executable: the
//!    candidate block ids (items, entities and the like are in there too).
//! 2. `plan <probes.tsv> <world-dir>` writes a behavior pack whose function places each probe
//!    (`block<TAB>states` per line) in its own cell, packed in stone so nothing falls or pops
//!    off. The server runs it and saves the world.
//! 3. `collect <world-dir> <probes.tsv> <out.tsv>` reads back what the game saved for each probe:
//!    the block it really made and its complete state.
//!
//! What the game refuses to place (an id that is not a block, a state or value it does not
//! have) stays stone, so the results are the game's own answer.

mod bedrock_finish;
mod bedrock_legacy;
mod blockmap;
mod entities;
mod items;
mod legacy_registry;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use mctk_bedrock::{BedrockWorld, ChunkKey, ChunkTag, Dimension, SubChunk};
use mctk_nbt::Tag;

/// Probe cells per row and column of a layer, and the spacing between them.
const CELLS: usize = 30;
const SPACING: i32 = 3;
/// Lowest probe layer and the ceiling; worlds from before 1.18 start at y 0 and end at 255
/// (MCTK_FIRST_Y and MCTK_MAX_Y set them).
fn env_i32(name: &str, default: i32) -> i32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
#[allow(non_snake_case)]
fn FIRST_Y() -> i32 {
    env_i32("MCTK_FIRST_Y", -58)
}
#[allow(non_snake_case)]
fn MAX_Y() -> i32 {
    env_i32("MCTK_MAX_Y", 300)
}
/// Commands per function file; the server runs at most 10000 per function.
const FUNCTION_LINES: usize = 9000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["strings", exe, out] => strings(Path::new(exe), Path::new(out)),
        ["blockmap-report"] => blockmap::report(),
        ["blockmap-compare"] => blockmap::compare(None),
        ["entity-plan", ids, out] => entities::plan(Path::new(ids), Path::new(out)),
        ["entity-read", world, out] => entities::read(Path::new(world), Path::new(out)),
        ["items-emit", out] => items::emit(Path::new(out)),
        ["legacy-registries", out] => legacy_registry::emit(Path::new(out)),
        ["bedrock-finish", registry, world, exe_ids, block_version, out, server] => block_version
            .parse::<i64>()
            .map_err(|_| "bad block version".to_string())
            .and_then(|v| bedrock_finish::finish(Path::new(registry), Path::new(world), Path::new(exe_ids), v, Path::new(out), Path::new(server))),

        ["blockmap-emit", out] => blockmap::emit(Path::new(out)),
        ["blockmap-compare", prefix] => blockmap::compare(Some(prefix)),
        ["legacy-gen", template, out] => bedrock_legacy::generate(Path::new(template), Path::new(out)),
        ["legacy-read", world, out] => bedrock_legacy::read(Path::new(world), Path::new(out)),
        ["plan", probes, world] => plan(Path::new(probes), Path::new(world)),
        ["collect", world, probes, out] => collect(Path::new(world), Path::new(probes), Path::new(out)),
        ["downgrades", r1, u1, r2, u2, registry, out] => {
            downgrades(&[(Path::new(r1), Path::new(u1)), (Path::new(r2), Path::new(u2))], Path::new(registry), Path::new(out))
        }
        ["probes2", p1, r1, out] =>probes2(Path::new(p1), Path::new(r1), Path::new(out)),
        ["audit", registry] => audit(Path::new(registry)),
        ["compare", a, b] => compare(Path::new(a), Path::new(b)),
        ["console-audit", dir] => console_audit(Path::new(dir)),
        ["biome-audit", ids] => biome_audit(Path::new(ids)),
        ["biome-ids", world, places, out] => biome_ids(Path::new(world), Path::new(places), Path::new(out)),
        ["archive-ids", archive, suffix, out] => archive_ids(Path::new(archive), suffix, Path::new(out)),
        ["add-ids", registry, entities, biomes, out] => {
            add_ids(Path::new(registry), Path::new(entities), Path::new(biomes), Path::new(out))
        }
        ["mapping-names", out] => names_from_mapping(Path::new(out)),
        ["probes3", registry, out] => probes3(Path::new(registry), Path::new(out)),
        ["extend", registry, p3, r3, out] => extend(Path::new(registry), Path::new(p3), Path::new(r3), Path::new(out)),
        ["build", p1, r1, p2, r2, version, out] => {
            build(Path::new(p1), Path::new(r1), Path::new(p2), Path::new(r2), version, Path::new(out))
        }
        _ => Err("usage: mctk-gen strings|plan|collect|probes2|build (see the module docs)".into()),
    };
    if let Err(e) = result {
        eprintln!("mctk-gen: {e}");
        std::process::exit(1);
    }
}

/// Every `minecraft:<lowercase, digits, underscores>` string in the file.
fn strings(exe: &Path, out: &Path) -> Result<(), String> {
    let data = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let needle = b"minecraft:";
    let mut ids = BTreeSet::new();
    let mut i = 0;
    while let Some(p) = data[i..].windows(needle.len()).position(|w| w == needle) {
        let start = i + p;
        let mut end = start + needle.len();
        while end < data.len() && matches!(data[end], b'a'..=b'z' | b'0'..=b'9' | b'_') {
            end += 1;
        }
        if end > start + needle.len() {
            ids.insert(String::from_utf8_lossy(&data[start..end]).into_owned());
        }
        i = end;
    }
    let text: String = ids.iter().map(|s| format!("{s}
")).collect();
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("{} candidate ids", ids.len());
    Ok(())
}

fn read_probes(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.split_once('\t') {
            Some((b, s)) => (b.to_string(), s.to_string()),
            None => (l.to_string(), String::new()),
        })
        .collect())
}

/// World position of probe `i`.
fn cell(i: usize) -> (i32, i32, i32) {
    let per_layer = CELLS * CELLS;
    let (layer, k) = (i / per_layer, i % per_layer);
    (
        (k % CELLS) as i32 * SPACING + 1,
        FIRST_Y() + layer as i32 * SPACING,
        (k / CELLS) as i32 * SPACING + 1,
    )
}

fn plan(probes: &Path, world: &Path) -> Result<(), String> {
    let probes = read_probes(probes)?;
    let layers = probes.len().div_ceil(CELLS * CELLS);
    if FIRST_Y() + layers as i32 * SPACING > MAX_Y() {
        return Err(format!("{} probes do not fit; split the list", probes.len()));
    }
    let span = CELLS as i32 * SPACING + 1;
    // Pack the whole box in stone, three layers of blocks per fill (under the 32768 limit).
    let top = FIRST_Y() + layers as i32 * SPACING;
    let mut lines = Vec::new();
    let mut y = FIRST_Y() - 2;
    while y <= top {
        let y2 = (y + 2).min(top);
        lines.push(format!("fill 0 {y} 0 {span} {y2} {span} minecraft:stone"));
        y += 3;
    }
    for (i, (block, states)) in probes.iter().enumerate() {
        let (x, y, z) = cell(i);
        // Older servers pop a plant that has nothing to stand on; MCTK_SUPPORT=dirt puts it there.
        if let Ok(support) = std::env::var("MCTK_SUPPORT") {
            lines.push(format!("setblock {x} {} {z} minecraft:{support}", y - 1));
        }
        lines.push(if states.is_empty() {
            format!("setblock {x} {y} {z} {block}")
        } else if let Some(data) = states.strip_prefix('#') {
            // The old numeric data value, which the game turns into the block's own states.
            format!("setblock {x} {y} {z} {block} {data}")
        } else {
            format!("setblock {x} {y} {z} {block} [{states}]")
        });
    }
    // One function run may execute at most 10000 commands, so the list is split in files.
    let pack = world.join("behavior_packs/probe");
    std::fs::create_dir_all(pack.join("functions")).map_err(|e| e.to_string())?;
    let files = lines.chunks(FUNCTION_LINES).count();
    for (n, chunk) in lines.chunks(FUNCTION_LINES).enumerate() {
        let mut f = chunk.join("\n");
        f.push('\n');
        std::fs::write(pack.join(format!("functions/p{n}.mcfunction")), f).map_err(|e| e.to_string())?;
    }
    let uuid = "7c3f2a10-5b1e-4d7a-9a55-0e1d2c3b4a59";
    let module = "1b9d6e44-2f0a-4c8e-8d3b-6a7f5e4d3c21";
    std::fs::write(
        pack.join("manifest.json"),
        format!(
            r#"{{"format_version":2,"header":{{"name":"probe","description":"mctk probe","uuid":"{uuid}","version":[1,0,0],"min_engine_version":[1,16,0]}},"modules":[{{"type":"data","uuid":"{module}","version":[1,0,0]}}]}}"#
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        world.join("world_behavior_packs.json"),
        format!(r#"[{{"pack_id":"{uuid}","version":[1,0,0]}}]"#),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "{} probes in {layers} layers, {files} functions (p0..p{}); tickingarea add 0 {} 0 {span} {top} {span} probe",
        probes.len(),
        files - 1,
        FIRST_Y() - 2
    );
    Ok(())
}

/// A state value's type and text as the game saves it: `b:1`, `i:3`, `s:top`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Value(String);

impl Value {
    /// The text a `setblock` state wants: `true`, `3` or `"top"`.
    fn command(&self) -> String {
        match self.0.split_once(':') {
            Some(("b", v)) => (v != "0").to_string(),
            Some(("i", v)) => v.to_string(),
            Some((_, v)) => format!("\"{v}\""),
            None => self.0.clone(),
        }
    }
}

/// Parses `key=b:1,key2=i:3` into ordered pairs.
fn parse_states(text: &str) -> Vec<(String, Value)> {
    text.split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_string(), Value(v.to_string())))
        })
        .collect()
}

/// Reads a results file into `(saved name, saved states)` per probe index.
fn read_results(path: &Path) -> Result<Vec<(String, Vec<(String, Value)>)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text
        .lines()
        .map(|l| {
            let mut c = l.split('\t');
            let _ = c.next();
            let name = c.next().unwrap_or("").to_string();
            (name, parse_states(c.next().unwrap_or("")))
        })
        .collect())
}

/// Candidate values for string states, from the mapping data already in the repo. They are only
/// things to try; the game accepts or refuses each one.
fn string_hints() -> std::collections::BTreeMap<String, BTreeSet<String>> {
    let mut hints: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    let m = mctk_mapping::mappings();
    for java in m.java_states() {
        let Some(b) = m.java_to_bedrock(java) else { continue };
        for (k, v) in &b.states {
            if let Tag::String(s) = v {
                hints.entry(k.clone()).or_default().insert(s.clone());
            }
        }
    }
    hints
}

/// Highest integer tried for an int state. Bedrock's largest known range (kelp and cave vine
/// age) is 25.
const INT_MAX_PROBE: i64 = 31;

/// Whether the game took a probe. A refused command leaves the stone the cell was packed in, so
/// anything else there means it was placed, even if the block then popped off (air), settled on
/// other states, or changed by itself (copper weathering, coral dying without water).
fn took(block: &str, saved: &str) -> bool {
    saved != "<missing>" && (saved != "minecraft:stone" || block == "minecraft:stone")
}

/// States the game's `setblock` refuses to place alone although they are valid in worlds: the
/// upper half of a door or a tall plant needs its lower half.
fn needs_partner(key: &str) -> bool {
    key == "upper_block_bit"
}

/// The states of blocks that pop off when placed alone (a bell, tall plants), which phase 1 could
/// not read back from the world: the first state the repo's mapping lists for them. The names and
/// values are only candidates; phase 2 still has the game accept or refuse each one.
fn popped_states() -> std::collections::BTreeMap<String, Vec<(String, Value)>> {
    let mut out: std::collections::BTreeMap<String, Vec<(String, Value)>> = Default::default();
    let m = mctk_mapping::mappings();
    for java in m.java_states() {
        let Some(b) = m.java_to_bedrock(java) else { continue };
        out.entry(b.name.clone()).or_insert_with(|| {
            b.states
                .iter()
                .map(|(k, v)| {
                    let t = match v {
                        Tag::Byte(n) => format!("b:{n}"),
                        Tag::Int(n) => format!("i:{n}"),
                        Tag::String(s) => format!("s:{s}"),
                        other => format!("?:{other}"),
                    };
                    (k.clone(), Value(t))
                })
                .collect()
        });
    }
    out
}

/// What phase 1 learned about a block: its saved states when it was placed as itself, the
/// mapping's candidates when it popped off (air), nothing when it was an alias of another block.
fn phase1_states(
    block: &str,
    saved: &str,
    states: &[(String, Value)],
    popped: &std::collections::BTreeMap<String, Vec<(String, Value)>>,
) -> Option<Vec<(String, Value)>> {
    if saved == block {
        Some(states.to_vec())
    } else if took(block, saved) {
        // Placed, but not as itself: it popped off (air) or the cell changed it (a coral that
        // died, nylium that decayed). The game's own state list could not be read back.
        popped.get(block).cloned()
    } else {
        None
    }
}

/// Phase 2: one probe per candidate value of every state of every block that phase 1 placed.
fn probes2(probes1: &Path, results1: &Path, out: &Path) -> Result<(), String> {
    let blocks = read_probes(probes1)?;
    let results = read_results(results1)?;
    let popped = popped_states();
    let hints = string_hints();
    let mut lines = String::new();
    let mut count = 0usize;
    // String values the game itself reported for a state name, on any block.
    let mut seen: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    for (name, states) in &results {
        let _ = name;
        for (k, v) in states {
            if let Some(s) = v.0.strip_prefix("s:") {
                seen.entry(k.clone()).or_default().insert(s.to_string());
            }
        }
    }
    for ((block, _), (saved, states)) in blocks.iter().zip(&results) {
        let Some(states) = phase1_states(block, saved, states, &popped) else {
            continue;
        };
        for (key, default) in &states {
            let mut values: BTreeSet<Value> = BTreeSet::new();
            match default.0.split_once(':') {
                Some(("b", _)) => {
                    values.insert(Value("b:0".into()));
                    values.insert(Value("b:1".into()));
                }
                Some(("i", _)) => {
                    for i in 0..=INT_MAX_PROBE {
                        values.insert(Value(format!("i:{i}")));
                    }
                }
                _ => {
                    for s in hints.get(key).into_iter().flatten().chain(seen.get(key).into_iter().flatten()) {
                        values.insert(Value(format!("s:{s}")));
                    }
                    values.insert(default.clone());
                }
            }
            for v in values {
                writeln!(lines, "{block}\t\"{key}\":{}", v.command()).unwrap();
                count += 1;
            }
        }
        // Older versions have string states whose values no table of ours lists (`dirt_type`,
        // `flower_type`); the game's own numeric data values lead to them.
        if states.iter().any(|(_, v)| v.0.starts_with("s:")) {
            for data in 1..=15 {
                writeln!(lines, "{block}\t#{data}").unwrap();
                count += 1;
            }
        }
    }
    std::fs::write(out, lines).map_err(|e| e.to_string())?;
    println!("{count} probes");
    Ok(())
}

/// For each `name<TAB>dimension<TAB>x<TAB>y<TAB>z` (a place the game said holds that biome),
/// reads the numeric biome id the game saved there. Dimension is 0 (overworld), 1 (nether) or
/// 2 (end). Writes `name<TAB>id` lines; a place whose chunk was not saved is left out.
fn biome_ids(world: &Path, places: &Path, out: &Path) -> Result<(), String> {
    use mctk_bedrock::{BiomeStorage, Data3D};
    let w = BedrockWorld::open(world).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(places).map_err(|e| format!("{}: {e}", places.display()))?;
    let mut result = String::new();
    let (mut found, mut missing) = (0, 0);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let c: Vec<&str> = line.split('\t').collect();
        let [name, dim, x, y, z] = c.as_slice() else { continue };
        let (dim, x, y, z): (i32, i32, i32, i32) = (
            dim.parse().map_err(|_| "bad dimension")?,
            x.parse().map_err(|_| "bad x")?,
            y.parse().map_err(|_| "bad y")?,
            z.parse().map_err(|_| "bad z")?,
        );
        let dimension = Dimension::from_id(dim).ok_or("unknown dimension")?;
        let key = ChunkKey { x: x.div_euclid(16), z: z.div_euclid(16), dimension, tag: ChunkTag::Data3D, subchunk: None };
        let Some(data) = w.db.get(&key.encode()).and_then(|b| Data3D::parse(b).ok()) else {
            missing += 1;
            continue;
        };
        // The overworld starts at y = -64; the other dimensions at 0.
        let min_y = if dim == 0 { -64 } else { 0 };
        let section = ((y - min_y).div_euclid(16)).clamp(0, data.biomes.len() as i32 - 1) as usize;
        // A "same as below" storage repeats the nearest one before it.
        let storage = (0..=section).rev().find_map(|i| match &data.biomes[i] {
            BiomeStorage::Palettized { palette, indices, .. } => Some((palette, indices)),
            BiomeStorage::CopyPrevious => None,
        });
        let Some((palette, indices)) = storage else {
            missing += 1;
            continue;
        };
        let i = mctk_bedrock::block_index(x.rem_euclid(16) as usize, y.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
        writeln!(result, "{name}\t{}", palette[indices[i] as usize]).unwrap();
        found += 1;
    }
    std::fs::write(out, result).map_err(|e| e.to_string())?;
    println!("{found} ids read, {missing} places had no saved chunk");
    Ok(())
}

/// The first run of digits in `s`.
fn first_number(s: &str) -> Option<u32> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let digits: String = s[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Reads the legacy console game's own registrations (`Minecraft.World/Tile.cpp` and
/// `EntityIO.cpp`) and reports which blocks and entities the converter's legacy tables do not
/// cover. Block ids and their data values come from the source; nothing here is guessed.
fn console_audit(world_dir: &Path) -> Result<(), String> {
    let legacy = mctk_mapping::legacy::legacy();
    let tiles = std::fs::read_to_string(world_dir.join("Tile.cpp")).map_err(|e| format!("Tile.cpp: {e}"))?;
    // `Tile::field = (...)(new Class(ID)...` on one line; commented-out lines do not count.
    let mut blocks: Vec<(u32, String)> = Vec::new();
    for line in tiles.lines() {
        let t = line.trim_start();
        if t.starts_with("//") || !t.starts_with("Tile::") {
            continue;
        }
        let Some((field, rest)) = t["Tile::".len()..].split_once('=') else { continue };
        let Some(after_new) = rest.split_once("new ").map(|(_, a)| a) else { continue };
        let Some((class, args)) = after_new.split_once('(') else { continue };
        if let Some(id) = first_number(args) {
            blocks.push((id, format!("{} ({})", field.trim(), class.trim())));
        }
    }
    blocks.sort();
    blocks.dedup_by_key(|(id, _)| *id);
    let (mut unmapped, mut partial) = (Vec::new(), Vec::new());
    for (id, what) in &blocks {
        let mapped = (0..16u8).filter(|d| legacy.block(*id as u16, *d).is_some()).count();
        match mapped {
            0 => unmapped.push(format!("{id} {what}")),
            n if n < 16 => partial.push(format!("{id} {what}: {n}/16 data values mapped")),
            _ => {}
        }
    }
    println!("{} console blocks registered; {} have no mapping at all, {} only some data values", blocks.len(), unmapped.len(), partial.len());
    for u in &unmapped {
        println!("  unmapped: {u}");
    }

    let entities = std::fs::read_to_string(world_dir.join("EntityIO.cpp")).map_err(|e| format!("EntityIO.cpp: {e}"))?;
    let mut missing = Vec::new();
    let mut total = 0;
    for line in entities.lines() {
        let t = line.trim_start();
        if t.starts_with("//") || !t.starts_with("setId(") {
            continue;
        }
        // setId(create, TYPE, L"Name", idNum ...)
        let Some((_, after)) = t.split_once("L\"") else { continue };
        let Some((name, rest)) = after.split_once('"') else { continue };
        let Some(id) = rest.split_once(',').and_then(|(_, r)| first_number(r)) else { continue };
        total += 1;
        if legacy.entity(name).is_none() {
            missing.push(format!("{id} {name}"));
        }
    }
    println!("{total} console entities registered; {} have no Java id in the converter's table", missing.len());
    for m in &missing {
        println!("  unmapped: {m}");
    }
    Ok(())
}

/// Prints each game-verified Bedrock biome (id read by `biome-ids`) with the Java biome the repo's
/// mapping turns that id into, and the id it gives back for that Java biome.
fn biome_audit(ids: &Path) -> Result<(), String> {
    let m = mctk_mapping::mappings();
    let text = std::fs::read_to_string(ids).map_err(|e| format!("{}: {e}", ids.display()))?;
    let (mut ok, mut bad) = (0, 0);
    let mut rows: Vec<(i32, String, String)> = Vec::new();
    for line in text.lines() {
        let Some((name, id)) = line.split_once('\t') else { continue };
        let id: i32 = id.trim().parse().map_err(|_| "bad id")?;
        let java = m.bedrock_biome_to_java(id).map(str::to_string);
        let back = java.as_deref().and_then(|j| m.java_biome_to_bedrock(j));
        let verdict = match (&java, back) {
            (Some(j), Some(b)) if b == id => {
                ok += 1;
                format!("{j}")
            }
            (Some(j), b) => {
                bad += 1;
                format!("{j}  (round trip gives {b:?})")
            }
            (None, _) => {
                bad += 1;
                "NO JAVA BIOME for this id".to_string()
            }
        };
        rows.push((id, name.trim_start_matches("minecraft:").to_string(), verdict));
    }
    rows.sort();
    for (id, name, v) in rows {
        println!("{id:>4}  {name:<26} -> {v}");
    }
    println!("{ok} ids map to a Java biome and back, {bad} do not");
    Ok(())
}

/// The file names stored in a Bedrock `.brarchive` (its table is plain text), as ids: for
/// `bamboo_jungle.biome.json` and suffix `.biome.json`, `minecraft:bamboo_jungle`.
fn archive_ids(archive: &Path, suffix: &str, out: &Path) -> Result<(), String> {
    let data = std::fs::read(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let text: String = data.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '\n' }).collect();
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        if let Some(stem) = line.strip_suffix(suffix) {
            if !stem.is_empty() && stem.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.')) {
                ids.insert(format!("minecraft:{stem}"));
            }
        }
    }
    let list: String = ids.iter().map(|i| format!("{i}
")).collect();
    std::fs::write(out, list).map_err(|e| e.to_string())?;
    println!("{} ids", ids.len());
    Ok(())
}

/// Puts the validated entity and biome ids into a registry file.
fn add_ids(registry: &Path, entities: &Path, biomes: &Path, out: &Path) -> Result<(), String> {
    let mut reg = mctk_registry::Registry::from_text(&std::fs::read_to_string(registry).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let read = |p: &Path| -> Result<Vec<String>, String> {
        let mut v: Vec<String> = std::fs::read_to_string(p)
            .map_err(|e| format!("{}: {e}", p.display()))?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        v.sort();
        v.dedup();
        Ok(v)
    };
    reg.entities = read(entities)?;
    reg.biomes = read(biomes)?;
    std::fs::write(out, reg.to_text()).map_err(|e| e.to_string())?;
    println!("{} entities, {} biomes", reg.entities.len(), reg.biomes.len());
    Ok(())
}

/// Compares every block of the subchunks two Bedrock worlds have in common (an overworld
/// conversion against the copy the game re-saved after loading it).
fn compare(a: &Path, b: &Path) -> Result<(), String> {
    let (wa, wb) = (
        BedrockWorld::open(a).map_err(|e| e.to_string())?,
        BedrockWorld::open(b).map_err(|e| e.to_string())?,
    );
    let (mut subchunks, mut blocks, mut differ) = (0u64, 0u64, 0u64);
    let mut kinds: std::collections::BTreeMap<String, u64> = Default::default();
    for (key, bytes) in &wa.db.entries {
        let mctk_bedrock::Key::Chunk(ck) = mctk_bedrock::Key::decode(key) else { continue };
        if ck.tag != ChunkTag::SubChunkPrefix {
            continue;
        }
        let (Some(other), Ok(sa)) = (wb.db.get(key), SubChunk::parse(bytes)) else { continue };
        let Ok(sb) = SubChunk::parse(other) else { continue };
        let (Some(pa), Some(pb)) = (sa.blocks(), sb.blocks()) else { continue };
        subchunks += 1;
        for i in 0..4096 {
            blocks += 1;
            let (ea, eb) = (&pa.palette[pa.indices[i] as usize], &pb.palette[pb.indices[i] as usize]);
            let fmt = |e: &mctk_nbt::NamedTag| {
                format!("{}[{}]", e.tag.get("name").and_then(Tag::as_str).unwrap_or("?"), states_text(e.tag.get("states")))
            };
            let (ta, tb) = (fmt(ea), fmt(eb));
            if ta != tb {
                differ += 1;
                *kinds.entry(format!("{ta}  ->  {tb}")).or_default() += 1;
            }
        }
    }
    println!("{subchunks} subchunks compared, {blocks} blocks, {differ} differ");
    let mut top: Vec<_> = kinds.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (k, n) in top.into_iter().take(25) {
        println!("{n:>8}  {k}");
    }
    Ok(())
}

/// Writes every Bedrock block name the repo's mapping knows: more candidate ids to try.
fn names_from_mapping(out: &Path) -> Result<(), String> {
    let m = mctk_mapping::mappings();
    let mut names = BTreeSet::new();
    for java in m.java_states() {
        if let Some(b) = m.java_to_bedrock(java) {
            names.insert(b.name.clone());
        }
    }
    let text: String = names.iter().map(|n| format!("{n}
")).collect();
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("{} names", names.len());
    Ok(())
}

/// Checks every Bedrock state the repo's Java mapping produces against a game-built registry.
fn audit(registry: &Path) -> Result<(), String> {
    use mctk_registry::{BlockCheck, StateProblem};
    let reg = mctk_registry::Registry::from_text(&std::fs::read_to_string(registry).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let m = mctk_mapping::mappings();
    let (mut total, mut exact) = (0usize, 0usize);
    let mut unknown_blocks: std::collections::BTreeMap<String, usize> = Default::default();
    let mut problems: std::collections::BTreeMap<String, usize> = Default::default();
    for java in m.java_states() {
        let Some(b) = m.java_to_bedrock(java) else { continue };
        total += 1;
        let props: Vec<(String, String)> = b
            .states
            .iter()
            .map(|(k, v)| {
                let text = match v {
                    Tag::Byte(n) => format!("b:{n}"),
                    Tag::Int(n) => format!("i:{n}"),
                    Tag::String(s) => format!("s:{s}"),
                    other => format!("?:{other}"),
                };
                (k.clone(), text)
            })
            .collect();
        match reg.check_block(&b.name, props.iter().map(|(k, v)| (k.as_str(), v.as_str()))) {
            BlockCheck::Exact => exact += 1,
            BlockCheck::UnknownBlock => *unknown_blocks.entry(b.name.clone()).or_default() += 1,
            BlockCheck::BadState(ps) => {
                for p in ps {
                    let key = match p {
                        StateProblem::UnknownProperty(k) => format!("{} has no state {k}", b.name),
                        StateProblem::BadValue { property, value } => {
                            format!("{}: {property}={value} refused", b.name)
                        }
                        StateProblem::MissingProperty(k) => format!("{} state {k} missing", b.name),
                    };
                    *problems.entry(key).or_default() += 1;
                }
            }
        }
    }
    println!("{total} mapped Bedrock states: {exact} exact in the game's registry");
    println!("blocks the game does not have: {}", unknown_blocks.len());
    for (b, n) in unknown_blocks.iter().take(80) {
        println!("  {n:>4}  {b}");
    }
    println!("state problems: {}", problems.len());
    for (p, n) in problems.iter().take(12) {
        println!("  {n:>4}  {p}");
    }
    Ok(())
}

/// Phase 3: int states whose accepted range reached the phase 2 limit are probed further.
fn probes3(registry: &Path, out: &Path) -> Result<(), String> {
    let reg = mctk_registry::Registry::from_text(&std::fs::read_to_string(registry).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut lines = String::new();
    let mut count = 0;
    for (block, def) in &reg.blocks {
        for (key, values) in &def.properties {
            let ints: Vec<i64> = values.iter().filter_map(|v| v.strip_prefix("i:")?.parse().ok()).collect();
            // A value at or past the phase 2 limit (the range reached it, or the block's own
            // default lies beyond it) means there may be more.
            if ints.iter().any(|&n| n >= INT_MAX_PROBE) {
                for n in (INT_MAX_PROBE + 1)..=255 {
                    if !ints.contains(&n) {
                        writeln!(lines, "{block}\t\"{key}\":{n}").unwrap();
                        count += 1;
                    }
                }
            }
        }
    }
    std::fs::write(out, lines).map_err(|e| e.to_string())?;
    println!("{count} probes");
    Ok(())
}

/// Adds the extra int values the game accepted in phase 3 to the registry.
fn extend(registry: &Path, probes3: &Path, results3: &Path, out: &Path) -> Result<(), String> {
    let mut reg = mctk_registry::Registry::from_text(&std::fs::read_to_string(registry).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let probes = read_probes(probes3)?;
    let results = read_results(results3)?;
    let mut added = 0;
    for ((block, state), (saved, _)) in probes.iter().zip(&results) {
        if !took(block, saved) {
            continue;
        }
        let Some((k, v)) = state.split_once("\":") else { continue };
        let key = k.trim_start_matches('"');
        if let Some(values) = reg.blocks.get_mut(block).and_then(|d| d.properties.get_mut(key)) {
            values.push(format!("i:{v}"));
            added += 1;
        }
    }
    for def in reg.blocks.values_mut() {
        for values in def.properties.values_mut() {
            if values.iter().all(|v| v.starts_with("i:")) {
                values.sort_by_key(|v| v[2..].parse::<i64>().unwrap_or(i64::MAX));
            }
        }
    }
    std::fs::write(out, reg.to_text()).map_err(|e| e.to_string())?;
    println!("{added} values added");
    Ok(())
}

/// Builds the Bedrock registry from both phases.
fn build(probes1: &Path, results1: &Path, probes2: &Path, results2: &Path, version: &str, out: &Path) -> Result<(), String> {
    use mctk_registry::{BlockDef, Edition, Registry};
    let blocks = read_probes(probes1)?;
    let r1 = read_results(results1)?;
    let p2 = read_probes(probes2)?;
    let r2 = read_results(results2)?;

    let popped = popped_states();
    // The type of each state, from the default state the game saved in phase 1.
    let mut kinds: std::collections::HashMap<(String, String), char> = Default::default();
    for ((block, _), (saved, states)) in blocks.iter().zip(&r1) {
        if let Some(states) = phase1_states(block, saved, states, &popped) {
            for (k, v) in &states {
                kinds.insert((block.clone(), k.clone()), v.0.chars().next().unwrap_or('s'));
            }
        }
    }
    // Legacy and alternate ids the game turns into another block when it parses them.
    let mut aliases = String::new();
    for ((block, _), (saved, _)) in blocks.iter().zip(&r1) {
        if saved != block && saved != "minecraft:air" && saved != "<missing>" {
            writeln!(aliases, "{block}\t{saved}").unwrap();
        }
    }
    std::fs::write(out.with_extension("aliases.tsv"), aliases).map_err(|e| e.to_string())?;
    // (block, state) -> values the game accepted. A refused command leaves the stone the cell
    // was packed in, so a probe is accepted when its block is there, whatever state the game
    // then settled on: some states are recalculated from the surroundings (redstone power,
    // stair corners, ...), which says nothing about whether the value is valid.
    let mut accepted: std::collections::BTreeMap<(String, String), BTreeSet<Value>> = Default::default();
    for ((block, state), (saved, saved_states)) in p2.iter().zip(&r2) {
        if !took(block, saved) {
            continue;
        }
        // A numeric data value: every state it settled on is a value the game has.
        if state.starts_with('#') {
            if saved == block {
                for (k, v) in saved_states {
                    if kinds.contains_key(&(block.clone(), k.clone())) {
                        accepted.entry((block.clone(), k.clone())).or_default().insert(v.clone());
                    }
                }
            }
            continue;
        }
        // `"key":value` back into the key and the value tried.
        let Some((k, v)) = state.split_once("\":") else { continue };
        let key = k.trim_start_matches('"');
        let Some(kind) = kinds.get(&(block.clone(), key.to_string())) else { continue };
        let want = v.trim_matches('"');
        let value = match kind {
            'b' => Value(format!("b:{}", (want == "true") as u8)),
            'i' => Value(format!("i:{want}")),
            _ => Value(format!("s:{want}")),
        };
        accepted.entry((block.clone(), key.to_string())).or_default().insert(value);
    }

    let mut reg = Registry::new(Edition::Bedrock, version);
    for ((block, _), (saved, states)) in blocks.iter().zip(&r1) {
        let Some(states) = phase1_states(block, saved, states, &popped) else {
            continue;
        };
        let mut def = BlockDef { properties: indexmap::IndexMap::new(), defaults: indexmap::IndexMap::new() };
        for (key, default) in &states {
            let mut values: Vec<Value> = accepted
                .get(&(block.clone(), key.clone()))
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            if !values.contains(default) {
                values.push(default.clone());
            }
            if needs_partner(key) {
                for v in ["b:0", "b:1"] {
                    if !values.iter().any(|x| x.0 == v) {
                        values.push(Value(v.into()));
                    }
                }
            }
            // Numeric order for ints, text order otherwise.
            values.sort_by_key(|v| (v.0.split_once(':').and_then(|(_, n)| n.parse::<i64>().ok()), v.0.clone()));
            def.properties.insert(key.clone(), values.into_iter().map(|v| v.0).collect());
            def.defaults.insert(key.clone(), default.0.clone());
        }
        reg.blocks.insert(block.clone(), def);
    }
    std::fs::write(out, reg.to_text()).map_err(|e| e.to_string())?;
    let with_states = reg.blocks.values().filter(|d| !d.properties.is_empty()).count();
    println!("{} blocks, {with_states} with states", reg.blocks.len());
    Ok(())
}

/// Writes `registry` to `out` with `D` lines: for every state the older game saved in a probe
/// world, what the newest game made of it when it loaded that world. Each pair of results files
/// is the probe world as the older game saved it and as the newest game saved it afterwards.
fn downgrades(pairs: &[(&Path, &Path)], registry: &Path, out: &Path) -> Result<(), String> {
    let mut reg = mctk_registry::Registry::from_text(&std::fs::read_to_string(registry).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let spec = |name: &str, states: &[(String, Value)]| -> String {
        let mut parts: Vec<String> = states.iter().map(|(k, v)| format!("{k}={}", v.0)).collect();
        parts.sort();
        format!("{name}|{}", parts.join(","))
    };
    let mut seen: std::collections::HashSet<String> = Default::default();
    reg.downgrades.clear();
    for (older, newest) in pairs {
        let (old, new) = (read_results(older)?, read_results(newest)?);
        for ((oname, ostates), (nname, nstates)) in old.iter().zip(&new) {
            if oname == "<missing>" || nname == "<missing>" {
                continue;
            }
            let (a, b) = (spec(nname, nstates), spec(oname, ostates));
            if a != b && seen.insert(a.clone()) {
                reg.downgrades.push((a, b));
            }
        }
    }
    // A grass block with nothing above it in the dark turns to dirt in the newest game before it can
    // be read back, so its renaming is never seen; it is the one block this happens to.
    if reg.blocks.contains_key("minecraft:grass") && !reg.blocks.contains_key("minecraft:grass_block") && !seen.contains("minecraft:grass_block|") {
        reg.downgrades.push(("minecraft:grass_block|".to_string(), "minecraft:grass|".to_string()));
    }
    std::fs::write(out, reg.to_text()).map_err(|e| e.to_string())?;
    println!("{} downgrades", reg.downgrades.len());
    Ok(())
}

/// `key=b:1,key2=i:3,key3=s:top` for a block state compound.
fn states_text(states: Option<&Tag>) -> String {
    let Some(Tag::Compound(c)) = states else { return String::new() };
    let mut parts = Vec::new();
    for (k, v) in c {
        let t = match v {
            Tag::Byte(b) => format!("b:{b}"),
            Tag::Int(i) => format!("i:{i}"),
            Tag::String(s) => format!("s:{s}"),
            other => format!("?:{other}"),
        };
        parts.push(format!("{k}={t}"));
    }
    parts.join(",")
}

fn collect(world: &Path, probes: &Path, out: &Path) -> Result<(), String> {
    let probes = read_probes(probes)?;
    let w = BedrockWorld::open(world).map_err(|e| e.to_string())?;
    let mut text = String::new();
    let mut cache: std::collections::HashMap<(i32, i32, i8), Option<SubChunk>> = Default::default();
    let mut versions: std::collections::BTreeMap<i64, usize> = Default::default();
    for (i, _) in probes.iter().enumerate() {
        let (x, y, z) = cell(i);
        let (cx, cz, sy) = (x.div_euclid(16), z.div_euclid(16), y.div_euclid(16) as i8);
        let sub = cache.entry((cx, cz, sy)).or_insert_with(|| {
            let key = ChunkKey {
                x: cx,
                z: cz,
                dimension: Dimension::Overworld,
                tag: ChunkTag::SubChunkPrefix,
                subchunk: Some(sy),
            };
            w.db.get(&key.encode()).and_then(|b| SubChunk::parse(b).ok())
        });
        let (name, states) = sub
            .as_ref()
            .and_then(SubChunk::blocks)
            .and_then(|s| {
                let idx = mctk_bedrock::block_index(
                    x.rem_euclid(16) as usize,
                    y.rem_euclid(16) as usize,
                    z.rem_euclid(16) as usize,
                );
                let entry = s.palette.get(*s.indices.get(idx)? as usize)?;
                let name = entry.tag.get("name")?.as_str()?.to_string();
                if let Some(v) = entry.tag.get("version").and_then(Tag::as_i64) {
                    *versions.entry(v).or_insert(0usize) += 1;
                }
                Some((name, states_text(entry.tag.get("states"))))
            })
            .unwrap_or_else(|| ("<missing>".into(), String::new()));
        writeln!(text, "{i}\t{name}\t{states}").unwrap();
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    println!("{} probes read", probes.len());
    if let Some((v, n)) = versions.iter().max_by_key(|(_, n)| **n) {
        println!("block state version {v} ({n} probes)");
    }
    Ok(())
}
