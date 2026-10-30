//! Prunes a world with a shape selection, its inverse and (Java/console) a Time layer,
//! then reopens each output and checks that exactly the expected chunks survived,
//! byte for byte, and that everything else was carried over.
//!
//! cargo run --release -p mctk-prune --example verify_prune -- <java|bedrock|console> <world> <scratch dir>

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use mctk_anvil::{ChunkPos, Region, RegionPos};
use mctk_bedrock::{BedrockWorld, Key};
use mctk_console::{ConsoleRegion, ConsoleSave};
use mctk_nbt::Tag;
use mctk_prune::{Comparison, Layer, Selection};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn shapes(centre: (f64, f64)) -> Selection {
    Selection {
        layers: vec![
            Layer::Split {
                x: centre.0,
                z: centre.1,
                angle: 30.0,
            },
            Layer::Circle {
                x: centre.0,
                z: centre.1,
                radius: 40.0,
            },
        ],
        inverted: false,
    }
}

fn mean(points: impl Iterator<Item = (i32, i32)>) -> (f64, f64) {
    let (mut sx, mut sz, mut n) = (0.0, 0.0, 0.0f64);
    for (x, z) in points {
        sx += x as f64 * 16.0 + 8.0;
        sz += z as f64 * 16.0 + 8.0;
        n += 1.0;
    }
    (sx / n.max(1.0), sz / n.max(1.0))
}

fn inhabited(tag: &Tag) -> Option<i64> {
    tag.get("InhabitedTime")
        .or_else(|| tag.get("Level")?.get("InhabitedTime"))
        .and_then(Tag::as_i64)
}

// ---------------------------------------------------------------- Java

fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p)
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf())
            }
        }
    }
    out
}

fn region_pos(rel: &Path) -> Option<RegionPos> {
    rel.file_name()?
        .to_str()
        .and_then(RegionPos::from_file_name)
}

fn check_java(src: &Path, out: &Path, sel: &Selection) -> Res<usize> {
    mctk_java::JavaWorld::open(out)?;
    let (mut kept, mut decisions) = (0, BTreeMap::new());
    let src_files = files(src);
    // Terrain first, so entity/POI chunks can follow its decisions.
    let mut regions: Vec<&PathBuf> = src_files
        .iter()
        .filter(|f| region_pos(f).is_some())
        .collect();
    regions.sort_by_key(|f| {
        f.parent()
            .and_then(Path::file_name)
            .is_none_or(|n| n != "region")
    });
    for rel in regions {
        let pos = region_pos(rel).unwrap();
        let terrain = rel
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "region");
        let dim = rel
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let source = Region::open(src.join(rel))?;
        let target = out.join(rel);
        let target = target
            .is_file()
            .then(|| Region::open(&target))
            .transpose()?;
        for (lx, lz) in source.chunks().collect::<Vec<_>>() {
            let cpos = pos.chunk(lx, lz);
            let raw = source.raw_chunk(lx, lz)?.unwrap();
            let expect = if terrain {
                let k = sel.keeps(
                    cpos,
                    if sel.uses_inhabited_time() {
                        inhabited(&raw.nbt()?.tag)
                    } else {
                        None
                    },
                );
                decisions.insert((dim.clone(), cpos), k);
                kept += k as usize;
                k
            } else {
                decisions
                    .get(&(dim.clone(), cpos))
                    .copied()
                    .unwrap_or_else(|| sel.keeps(cpos, None))
            };
            let got = target
                .as_ref()
                .map(|t| t.raw_chunk(lx, lz))
                .transpose()?
                .flatten();
            match (expect, got) {
                (true, Some(g)) if g.data == raw.data => {}
                (false, None) => {}
                (e, g) => {
                    return Err(format!(
                        "{} chunk {cpos:?}: expected kept={e}, got present={}",
                        rel.display(),
                        g.is_some()
                    )
                    .into());
                }
            }
        }
    }
    for rel in src_files
        .iter()
        .filter(|f| region_pos(f).is_none() && !f.to_string_lossy().ends_with(".mcc"))
    {
        if std::fs::read(src.join(rel))? != std::fs::read(out.join(rel))? {
            return Err(format!("{} not copied", rel.display()).into());
        }
    }
    Ok(kept)
}

fn java(src: &Path, scratch: &Path) -> Res<()> {
    let centre = mean_of_overworld_java(src)?;
    let timed = run_java(src, scratch, "java-time", &time())?;
    let normal = run_java(src, scratch, "java-normal", &shapes(centre))?;
    let mut inv = shapes(centre);
    inv.inverted = true;
    let inverted = run_java(src, scratch, "java-inverted", &inv)?;
    println!(
        "Java: normal {normal} + inverted {inverted} terrain chunks kept (time layer kept {timed})"
    );
    Ok(())
}

fn mean_of_overworld_java(src: &Path) -> Res<(f64, f64)> {
    let world = mctk_java::JavaWorld::open(src)?;
    let ow = world
        .dimension("minecraft:overworld")
        .ok_or("no overworld")?;
    let mut points = Vec::new();
    for rpos in ow.regions()? {
        let r = Region::open(ow.region_dir().join(rpos.file_name()))?;
        points.extend(
            r.chunks()
                .map(|(lx, lz)| rpos.chunk(lx, lz))
                .map(|c| (c.x, c.z)),
        );
    }
    Ok(mean(points.into_iter()))
}

fn run_java(src: &Path, scratch: &Path, name: &str, sel: &Selection) -> Res<usize> {
    let out = scratch.join(name);
    let report = mctk_prune::prune_java(src, &out, sel, None)?;
    let kept = check_java(src, &out, sel)?;
    assert_eq!(kept, report.kept);
    println!(
        "  {name}: kept {}, removed {}: verified",
        report.kept, report.removed
    );
    Ok(kept)
}

fn time() -> Selection {
    Selection {
        layers: vec![Layer::Time {
            comparison: Comparison::Greater,
            ticks: 0,
        }],
        inverted: false,
    }
}

// ---------------------------------------------------------------- Bedrock

fn bedrock(src: &Path, scratch: &Path) -> Res<()> {
    let world = BedrockWorld::open(src)?;
    let centre = mean(
        world
            .db
            .entries
            .keys()
            .filter_map(|k| match Key::decode(k) {
                Key::Chunk(c) if c.dimension.id() == 0 => Some((c.x, c.z)),
                _ => None,
            }),
    );
    let mut counts = Vec::new();
    for (name, inverted) in [("bedrock-normal", false), ("bedrock-inverted", true)] {
        let mut sel = shapes(centre);
        sel.inverted = inverted;
        let out = scratch.join(name);
        let report = mctk_prune::prune_bedrock_world(src, &out, &sel, None)?;
        let pruned = BedrockWorld::open(&out)?;
        let mut removed_actors = HashSet::new();
        for (k, v) in &world.db.entries {
            if let Key::ActorDigest { x, z, .. } = Key::decode(k) {
                if !sel.keeps(ChunkPos::new(x, z), None) {
                    removed_actors.extend(v.chunks_exact(8).map(<[u8]>::to_vec));
                }
            }
        }
        for (k, v) in &world.db.entries {
            let expect = match Key::decode(k) {
                Key::Chunk(c) => sel.keeps(ChunkPos::new(c.x, c.z), None),
                Key::ActorDigest { x, z, .. } => sel.keeps(ChunkPos::new(x, z), None),
                Key::Actor(uid) => !removed_actors.contains(&uid),
                Key::Other(_) => true,
            };
            match (expect, pruned.db.get(k)) {
                (true, Some(g)) if g == v.as_slice() => {}
                (false, None) => {}
                (e, g) => {
                    return Err(format!(
                        "key {k:02x?}: expected kept={e}, got present={}",
                        g.is_some()
                    )
                    .into());
                }
            }
        }
        if pruned
            .db
            .entries
            .keys()
            .any(|k| !world.db.entries.contains_key(k))
        {
            return Err("pruned database has new keys".into());
        }
        if pruned.level.encode()? != world.level.encode()? {
            return Err("level.dat changed".into());
        }
        println!(
            "  {name}: kept {}, removed {} columns, {:?}: verified",
            report.kept, report.removed, report.notes
        );
        counts.push(report.kept);
    }
    println!(
        "Bedrock: normal {} + inverted {} columns kept",
        counts[0], counts[1]
    );
    Ok(())
}

// ---------------------------------------------------------------- Console

/// (dimension, x, z) -> (decompressed chunk, InhabitedTime)
type ConsoleChunks = BTreeMap<(i32, i32, i32), (Vec<u8>, Option<i64>)>;

fn console_chunks(save: &ConsoleSave) -> Res<ConsoleChunks> {
    let mut out = BTreeMap::new();
    for (dim, rx, rz, e) in save.archive.regions() {
        if e.data.is_empty() {
            continue;
        }
        let region = ConsoleRegion::parse(e.data.clone())?;
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            let raw = region.raw_chunk(lx, lz)?.unwrap();
            let bytes = raw.decompress()?;
            let nbt = raw.nbt()?;
            let (x, z) = (rx * 32 + lx as i32, rz * 32 + lz as i32);
            let level = nbt.tag.get("Level").ok_or("no Level")?;
            let nbt_pos = (
                level.get("xPos").and_then(Tag::as_i64),
                level.get("zPos").and_then(Tag::as_i64),
            );
            if nbt_pos != (Some(x as i64), Some(z as i64)) {
                return Err(format!("chunk position mismatch at {x},{z}: {nbt_pos:?}").into());
            }
            out.insert((dim, x, z), (bytes, inhabited(&nbt.tag)));
        }
    }
    Ok(out)
}

fn console(src: &Path, scratch: &Path) -> Res<()> {
    let save = ConsoleSave::open(src)?;
    let source = console_chunks(&save)?;
    let times: Vec<i64> = source.values().filter_map(|v| v.1).collect();
    println!(
        "  {} of {} chunks record InhabitedTime (max {:?})",
        times.len(),
        source.len(),
        times.iter().max()
    );
    let centre = mean(source.keys().filter(|k| k.0 == 0).map(|k| (k.1, k.2)));
    let mut inverted = shapes(centre);
    inverted.inverted = true;
    let tag = if save.platform == mctk_console::Platform::Ps3 {
        "ps3"
    } else {
        "x360"
    };
    for (name, sel) in [
        ("normal", shapes(centre)),
        ("inverted", inverted),
        ("time", time()),
    ] {
        let mut copy = save.clone();
        let result = mctk_prune::prune_console(&mut copy, &sel, None);
        if sel.uses_inhabited_time() && times.is_empty() {
            match result {
                Err(e) => println!("  {tag} {name}: refused as expected ({e})"),
                Ok(_) => return Err("Time layer accepted on a save without InhabitedTime".into()),
            }
            continue;
        }
        let report = result?;
        let out = scratch.join(format!("console-{tag}-{name}"));
        mctk_prune::save_console_copy(&copy, &out)?;
        let pruned = console_chunks(&ConsoleSave::open(&out)?)?;
        let mut kept = 0;
        for (key, (bytes, time)) in &source {
            let expect = sel.keeps(ChunkPos::new(key.1, key.2), *time);
            kept += expect as usize;
            match (expect, pruned.get(key)) {
                (true, Some((b, _))) if b == bytes => {}
                (false, None) => {}
                (e, g) => {
                    return Err(format!(
                        "chunk {key:?}: expected kept={e}, got present={}",
                        g.is_some()
                    )
                    .into());
                }
            }
        }
        if pruned.len() != kept || kept != report.kept {
            return Err("chunk count mismatch".into());
        }
        let others = |s: &ConsoleSave| -> Vec<(String, Vec<u8>)> {
            s.archive
                .entries
                .iter()
                .filter(|e| !e.name.ends_with(".mcr"))
                .map(|e| (e.name.clone(), e.data.clone()))
                .collect()
        };
        if others(&ConsoleSave::open(&out)?) != others(&save) {
            return Err("non-region files changed".into());
        }
        println!(
            "  {tag} {name}: kept {}, removed {}: verified",
            report.kept, report.removed
        );
    }
    Ok(())
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: <java|bedrock|console> <world> <scratch dir>";
    let (kind, src, scratch) = (
        args.get(1).ok_or(usage)?,
        Path::new(args.get(2).ok_or(usage)?),
        Path::new(args.get(3).ok_or(usage)?),
    );
    match kind.as_str() {
        "java" => java(src, scratch),
        "bedrock" => bedrock(src, scratch),
        "console" => console(src, scratch),
        _ => Err(usage.into()),
    }
}
